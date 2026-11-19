//! One active hosted viewport. All native renderer calls are scoped by the
//! actual BodyKey/host revision before entering the existing renderer.
use super::*;
use crate::history::SurfacePosition;
use crossterm::event::{KeyCode, KeyModifiers};
use mdtui3::{
    BodyId, BodyKey, BodyRendererClock, BodySpan, BodyView, FootnoteId, FootnoteReferenceId,
    FootnoteTarget,
};
use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet};
#[path = "footnote_surface/chooser.rs"]
mod chooser;
#[path = "footnote_surface/links.rs"]
mod links;
use links::{NoteLink, NoteLinkResult};

type Order = (bool, usize, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReturnAddress {
    Root(mdtui3::DocumentId, usize),
    Body(BodySpan),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum OccurrenceKey {
    Root(mdtui3::DocumentId, FootnoteReferenceId),
    Body(BodyId<FootnoteReferenceId>),
}
#[derive(Clone, Copy)]
struct Occurrence {
    key: OccurrenceKey,
    address: ReturnAddress,
}
#[derive(Default)]
struct ColdReferences {
    root: usize,
    root_done: bool,
    symbol: usize,
    local: usize,
    examined: bool,
    mapping: Option<(Occurrence, usize, mdtui3::BodyMapTask, Option<usize>)>,
}
#[derive(Clone, Copy)]
struct PendingMove {
    from: SurfacePosition,
    to: SurfacePosition,
    restore: bool,
    origin: Option<ReturnAddress>,
    pop_origin: bool,
    root_existing: bool,
    prepared: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct SurfaceMove {
    pub from: SurfacePosition,
    pub to: SurfacePosition,
    pub moved: bool,
    pub history_restore: bool,
}
/// Read-only note reading state. Source ranges are always BodyKey qualified;
/// the native selection API cannot be called with the host document here.
pub struct FootnoteReading<'a> {
    body: BodyKey,
    tools: &'a crate::reading::ReadingTools,
    status: &'a crate::reading::ReadingTools,
}
impl FootnoteReading<'_> {
    pub fn body(&self) -> BodyKey {
        self.body
    }
    pub fn selection(&self) -> Option<BodySpan> {
        self.tools.selection.range().map(|span| BodySpan {
            body: self.body,
            span,
        })
    }
    pub fn cursor(&self) -> Option<BodySpan> {
        self.tools.selection.cursor().map(|span| BodySpan {
            body: self.body,
            span,
        })
    }
    pub fn current_match(&self) -> Option<mdtui3::BodyDisplayMatch> {
        self.tools
            .current_match()
            .map(|value| mdtui3::BodyDisplayMatch {
                body: self.body,
                source: BodySpan {
                    body: self.body,
                    span: value.source,
                },
                display: value.display.clone(),
            })
    }
    pub fn query(&self) -> &str {
        self.tools.query()
    }
    pub fn message(&self) -> &str {
        self.status.message()
    }
    pub fn copy_pending(&self) -> bool {
        self.status.copy_pending()
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct FootnoteSurfaceStats {
    pub input_events: usize,
    pub input_mapping: WorkLimit,
    pub bootstrap: WorkLimit,
    pub retirement: WorkLimit,
    pub note_creations: usize,
    pub note_retirements: usize,
    pub active_owner: Option<BodyKey>,
    pub source_occurrences: usize,
    pub shape: super::super::shape::ShapeWork,
    pub objects: super::super::objects::ObjectStats,
    pub height_capacity_bytes: usize,
    pub checkpoint_capacity_bytes: usize,
    pub table_capacity_bytes: usize,
    pub math_owners: usize,
    pub math_retirement_jobs: usize,
    pub saved_intents: usize,
    pub saved_intent_records: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct NoteFoldState {
    pub id: BodyId<mdtui3::FoldId>,
    pub source: BodySpan,
    pub hidden: BodySpan,
    pub collapsed: bool,
    pub preview: Option<BodyId<mdtui3::FoldId>>,
    pub defaults: mdtui3::FoldDefaults,
    pub version: u64,
}
fn add_work(total: &mut WorkLimit, before: WorkLimit, after: WorkLimit) {
    total.queries += after.queries - before.queries;
    total.objects += after.objects - before.objects;
    total.bytes += after.bytes - before.bytes;
    total.tiles += after.tiles - before.tiles;
    total.recovery += after.recovery - before.recovery;
    total.dependencies += after.dependencies - before.dependencies;
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum NoteObject {
    Footnote(BodyId<FootnoteReferenceId>, BodySpan),
    Link(BodyId<LinkId>, BodySpan),
    Fold(BodyId<mdtui3::FoldId>, BodySpan),
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum NoteObjectKey {
    Footnote(BodyId<FootnoteReferenceId>),
    Link(BodyId<LinkId>),
    Fold(BodyId<mdtui3::FoldId>),
}
impl NoteObject {
    fn key(self) -> NoteObjectKey {
        match self {
            Self::Footnote(id, _) => NoteObjectKey::Footnote(id),
            Self::Link(id, _) => NoteObjectKey::Link(id),
            Self::Fold(id, _) => NoteObjectKey::Fold(id),
        }
    }
    fn source(self) -> BodySpan {
        match self {
            Self::Footnote(_, s) | Self::Link(_, s) | Self::Fold(_, s) => s,
        }
    }
}

pub(super) struct NoteViewport {
    key: BodyKey,
    clock: BodyRendererClock,
    binding: usize,
    shape: super::super::shape::SharedShape,
    viewport: ViewportState,
    reading: crate::reading::ReadingTools,
    retiring: bool,
    hits: Vec<NoteObject>,
    focus: Option<NoteObject>,
    fold_ui: crate::fold_ui::FoldUi,
    pending_target: Option<BodySpan>,
    refiner: Option<SourceRefiner>,
    raw_map: Option<mdtui3::BodyMapTask>,
    raw_range: Option<mdtui3::Span>,
    raw_copy: crate::reading::ReadingTools,
    raw_requested: bool,
    restore_offset: Option<u64>,
    reveal_task: Option<mdtui3::FoldNavigation>,
    revealed: Option<(u64, u64, usize)>,
    navigator: Navigator,
    link: Option<NoteLink>,
    queued_link: Option<BodyId<LinkId>>,
    anchors: Option<mdtui3::HeadingAnchors>,
    retire_anchors: bool,
    note_status: Option<(BodyId<FootnoteReferenceId>, u64, String)>,
    focus_fold_version: u64,
    paint: Option<(PaintGeneration, ViewportFrame)>,
    intent_capture: Option<mdtui3::FoldIntentCapture>,
    saved_fold: Option<mdtui3::FoldViewIntent>,
    restoring: Option<mdtui3::FoldViewIntent>,
}
struct NoteIntent {
    fold: mdtui3::FoldViewIntent,
    ui: crate::fold_ui::FoldUi,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct PaintGeneration {
    body: BodyKey,
    host: u64,
    parser: u64,
    binding: usize,
    width: u16,
    rows: u16,
    fold: u64,
    position: TopPosition,
}
impl NoteViewport {
    fn new(body: BodyView<'_>, width: u16, rows: u16) -> Result<Self, FrameError> {
        let key = body.key();
        let (viewport, shape) = body.with_renderer(key, body.host_revision(), |native| {
            let viewport = ViewportState::new(
                native,
                ViewportConfig {
                    width,
                    rows: rows.max(1),
                    ..Default::default()
                },
            )?;
            let shape = super::super::shape::SharedShape::from_current(native);
            let links = viewport.shared_links();
            links.set_shape(shape.clone())?;
            links.set_seed(super::super::shape::ShapeSeed::indexed(native));
            links.enable_footnote_numbers();
            Ok::<_, FrameError>((viewport, shape))
        })??;
        Ok(Self {
            key,
            clock: BodyRendererClock::new(body),
            binding: body.binding_epoch(),
            shape,
            viewport,
            reading: Default::default(),
            retiring: false,
            hits: vec![],
            focus: None,
            fold_ui: Default::default(),
            pending_target: None,
            refiner: None,
            raw_map: None,
            raw_range: None,
            raw_copy: Default::default(),
            raw_requested: false,
            restore_offset: None,
            reveal_task: None,
            revealed: None,
            navigator: Navigator::new(),
            link: None,
            queued_link: None,
            anchors: None,
            retire_anchors: false,
            note_status: None,
            focus_fold_version: 0,
            paint: None,
            intent_capture: None,
            saved_fold: None,
            restoring: None,
        })
    }
    fn apply(&mut self, body: BodyView<'_>, update: &Update) -> Result<(), FrameError> {
        if self.retiring {
            return Ok(());
        }
        self.clock.validate(body, update)?;
        self.paint = None;
        if self.raw_map.take().is_some() {
            self.raw_copy.cancel(&self.viewport.shared_links());
        }
        if let Some(local) = self.clock.apply(body, update)? {
            body.with_renderer(self.key, body.host_revision(), |native| {
                self.shape.apply(native, &local.value)?;
                if self.binding != body.binding_epoch() {
                    self.shape.invalidate_host_binding();
                    self.binding = body.binding_epoch();
                }
                self.viewport.apply_update(native, &local.value)?;
                self.fold_ui
                    .on_update(native, self.viewport.folds(), &local.value)?;
                self.refiner = None;
                self.reveal_task = None;
                self.revealed = None;
                self.reading.changed(native.revision());
                self.retire_anchors = true;
                self.note_status = None;
                if let Some(link) = &mut self.link {
                    link.cancelled = true;
                }
                Ok::<_, FrameError>(())
            })??;
        }
        Ok(())
    }
    fn retire(
        &mut self,
        body: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<Completion, FrameError> {
        body.with_renderer(self.key, body.host_revision(), |_| ())?;
        if !self.restore_choices(body, budget)? {
            return Ok(Completion::BudgetExhausted);
        }
        if !self.retiring
            && !body.with_renderer(self.key, body.host_revision(), |native| {
                let ready =
                    self.fold_ui
                        .settle_table_rows(native, self.viewport.folds_mut(), budget);
                if self.fold_ui.take_changed() {
                    self.viewport.fold_changed(native);
                }
                ready
            })?
        {
            return Ok(Completion::BudgetExhausted);
        }
        if !self.retiring
            && self.viewport.checkpoints().maintenance_in_progress()
            && body
                .with_renderer(self.key, body.host_revision(), |native| {
                    self.viewport.maintain_checkpoints(native, budget)
                })??
                .completion
                == Completion::BudgetExhausted
        {
            return Ok(Completion::BudgetExhausted);
        }
        if !self.retiring {
            self.retiring = true;
            self.paint = None;
            self.intent_capture = Some(self.viewport.folds().capture_intent());
            self.reading.cancel(&self.viewport.shared_links());
            self.raw_copy.cancel(&self.viewport.shared_links());
            self.shape.invalidate_host_binding();
        }
        if let Some(capture) = &mut self.intent_capture {
            if capture.advance(self.viewport.folds(), budget)? != Completion::Covered {
                return Ok(Completion::BudgetExhausted);
            }
            self.saved_fold = Some(
                self.intent_capture
                    .take()
                    .unwrap()
                    .finish()
                    .map_err(|_| LayoutError::OutOfOrderUpdate)?,
            );
        }
        if let Some(link) = &mut self.link
            && link.retire(budget) != Completion::Covered
        {
            return Ok(Completion::BudgetExhausted);
        }
        self.link = None;
        if let Some(anchors) = &mut self.anchors
            && anchors.retire(budget) != Completion::Covered
        {
            return Ok(Completion::BudgetExhausted);
        }
        self.anchors = None;
        if let Some(refiner) = &mut self.refiner
            && !refiner.retire(budget)
        {
            return Ok(Completion::BudgetExhausted);
        }
        self.refiner = None;
        if !self.fold_ui.suspend(budget) {
            return Ok(Completion::BudgetExhausted);
        }
        if !body.with_renderer(self.key, body.host_revision(), |native| {
            self.viewport.retire(native, budget)
        })?? {
            return Ok(Completion::BudgetExhausted);
        }
        if !self.viewport.shared_links().math().retire_all(budget) {
            return Ok(Completion::BudgetExhausted);
        }
        if self.viewport.shared_links().retire_current(budget) != Completion::Covered {
            return Ok(Completion::BudgetExhausted);
        }
        Ok(self.shape.retire_index(budget))
    }
    fn restore_choices(
        &mut self,
        body: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<bool, FrameError> {
        if let Some(intent) = &mut self.restoring {
            let completion = body.with_renderer(self.key, body.host_revision(), |native| {
                intent.restore(native, self.viewport.folds_mut(), budget)
            })??;
            if completion != Completion::Covered {
                return Ok(false);
            }
            self.restoring = None;
            body.with_renderer(self.key, body.host_revision(), |native| {
                self.fold_ui.resume(
                    native,
                    self.viewport.folds().defaults().paragraphs.is_some(),
                );
                self.viewport.fold_changed(native);
            })?;
        }
        Ok(true)
    }
}

#[derive(Default)]
pub(super) struct FootnoteUi {
    pub(super) open: bool,
    chooser: bool,
    selected: Option<FootnoteId>,
    entries: BTreeMap<Order, FootnoteId>,
    positions: BTreeMap<FootnoteId, Order>,
    seeded: usize,
    active: Option<Box<NoteViewport>>,
    origins: Vec<ReturnAddress>,
    notice: Option<String>,
    reveal: Option<BodySpan>,
    references: BTreeMap<(usize, usize), Occurrence>,
    reference_positions: HashMap<OccurrenceKey, (usize, usize)>,
    backlinks: Option<FootnoteId>,
    selected_reference: Option<(usize, usize)>,
    revision: Option<u64>,
    published: Option<SurfacePosition>,
    pending_move: Option<PendingMove>,
    move_serial: u64,
    move_result: Option<SurfaceMove>,
    cold: Option<ColdReferences>,
    external: Option<(u64, mdtui3::BodyObject<crate::external::ExternalTarget>)>,
    screen_body: Option<BodyKey>,
    work: FootnoteSurfaceStats,
    intents: HashMap<BodyKey, NoteIntent>,
    chooser_build: Option<chooser::ChooserBuild>,
    chooser_paint: Option<(chooser::ChooserGeneration, ViewportFrame)>,
    chooser_offset: usize,
}
impl FootnoteUi {
    pub(super) fn adopt(&mut self) {
        self.cold = Some(ColdReferences::default());
    }
    /// Adopt input-maintained occurrence indexes, never the hidden body text.
    /// One cursor and one mapping task survive a micro-budget yield. The only
    /// symbol walk is this initial adoption; future input uses actual deltas.
    pub(super) fn bootstrap(
        &mut self,
        document: &mut Utf8Document,
        budget: &mut Budget,
    ) -> Result<bool, FrameError> {
        let Some(mut cold) = self.cold.take() else {
            return Ok(true);
        };
        let before = budget.used();
        let result = self.bootstrap_inner(document, budget, &mut cold);
        add_work(&mut self.work.bootstrap, before, budget.used());
        if !matches!(result, Ok(true)) {
            self.cold = Some(cold);
        }
        result
    }
    fn bootstrap_inner(
        &mut self,
        host: &mut Utf8Document,
        budget: &mut Budget,
        cold: &mut ColdReferences,
    ) -> Result<bool, FrameError> {
        let document = host.document();
        while !cold.root_done {
            let mut cursor = document
                .footnote_references_in(mdtui3::Span::new(cold.root, document.source().len()))
                .ok_or(LayoutError::InvalidRange)?;
            match cursor.advance(budget) {
                Err(_) => return Ok(false),
                Ok(None) => cold.root_done = true,
                Ok(Some((id, reference))) => {
                    cold.root = reference.source.end;
                    let key = OccurrenceKey::Root(document.id(), id);
                    self.remove_reference(key);
                    self.insert_reference(
                        (reference.symbol.0, reference.source.start),
                        Occurrence {
                            key,
                            address: ReturnAddress::Root(document.id(), reference.source.start),
                        },
                    );
                }
            }
        }
        while cold.symbol < host.document().footnotes().len() {
            let document = host.document();
            if let Some((occurrence, symbol, map, physical)) = &mut cold.mapping {
                let ReturnAddress::Body(source) = occurrence.address else {
                    unreachable!()
                };
                let body = document
                    .footnote_body(source.body.symbol())
                    .ok_or(LayoutError::InvalidRange)?;
                loop {
                    match map.advance(body, budget)? {
                        mdtui3::BodyMapStep::BudgetExhausted => return Ok(false),
                        mdtui3::BodyMapStep::Mapping(m) => {
                            *physical =
                                Some(physical.map_or(m.original.start, |p| p.min(m.original.start)))
                        }
                        mdtui3::BodyMapStep::Complete => break,
                    }
                }
                let at = physical.ok_or(LayoutError::InvalidRange)?;
                let occurrence = *occurrence;
                let symbol = *symbol;
                cold.local = source.span.end;
                cold.mapping = None;
                self.remove_reference(occurrence.key);
                self.insert_reference((symbol, at), occurrence);
                continue;
            }
            if !cold.examined {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(false);
                }
                if let Some(body) = document.footnote_body(FootnoteId(cold.symbol)) {
                    let key = body.key();
                    host.subscribe_footnote_body_shape(key)?;
                }
                cold.examined = true;
            }
            let Some(body) = host.document().footnote_body(FootnoteId(cold.symbol)) else {
                cold.symbol += 1;
                cold.local = 0;
                cold.examined = false;
                continue;
            };
            let mut cursor = body.footnote_references_in(BodySpan {
                body: body.key(),
                span: mdtui3::Span::new(cold.local, body.range().span.end),
            })?;
            match cursor.advance(budget) {
                Err(_) => return Ok(false),
                Ok(None) => {
                    cold.symbol += 1;
                    cold.local = 0;
                    cold.examined = false;
                }
                Ok(Some((id, reference))) => {
                    let occurrence = Occurrence {
                        key: OccurrenceKey::Body(id),
                        address: ReturnAddress::Body(reference.source),
                    };
                    cold.mapping = Some((
                        occurrence,
                        reference.symbol.0,
                        body.map(mdtui3::BodyAddress::Projected(reference.source))?,
                        None,
                    ));
                }
            }
        }
        Ok(true)
    }
    fn finish_move(&mut self, to: SurfacePosition) {
        if let Some(pending) = self.pending_move.take() {
            let moved = pending.from != to;
            if moved {
                if let Some(origin) = pending.origin {
                    self.origins.push(origin);
                }
                if pending.pop_origin {
                    self.origins.pop();
                }
            }
            self.move_serial += 1;
            self.move_result = Some(SurfaceMove {
                from: pending.from,
                to,
                moved,
                history_restore: pending.restore,
            });
        }
        self.published = Some(to);
    }
    fn fail_move(&mut self) {
        if let Some(pending) = self.pending_move.take() {
            self.move_serial += 1;
            self.move_result = Some(SurfaceMove {
                from: pending.from,
                to: pending.from,
                moved: false,
                history_restore: pending.restore,
            });
        }
    }
    pub(super) fn reading(&self) -> Option<&crate::reading::ReadingTools> {
        if !self.open || self.chooser {
            return None;
        }
        self.active.as_ref().map(|a| {
            if a.raw_requested {
                &a.raw_copy
            } else {
                &a.reading
            }
        })
    }
    pub(super) fn pending(&self) -> bool {
        self.active.as_ref().is_some_and(|a| {
            a.raw_copy.copy_pending()
                || a.reading.copy_pending()
                || self.open
                    && !self.chooser
                    && (a.retiring
                        || a.fold_ui.pending()
                        || a.pending_target.is_some()
                        || a.raw_map.is_some()
                        || a.link.is_some()
                        || a.queued_link.is_some())
        })
    }
    pub(super) fn initializing(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| !a.retiring && !a.viewport.shared_links().shape_ready())
    }
    pub(super) fn input_busy(&self) -> bool {
        self.initializing()
            || self.open
                && !self.chooser
                && self
                    .active
                    .as_ref()
                    .is_some_and(|a| a.fold_ui.active() || a.restoring.is_some())
    }
    pub(super) fn initialize_native(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, FrameError> {
        if let Some(active) = &mut self.active
            && !active.retiring
        {
            if active.key.document() != document.id() {
                return Err(LayoutError::WrongDocument.into());
            }
            let body = document
                .footnote_body(active.key.symbol())
                .ok_or(LayoutError::InvalidRange)?;
            return Ok(
                body.with_renderer(active.key, body.host_revision(), |native| {
                    active.viewport.initialize(native, budget)
                })??,
            );
        }
        Ok(true)
    }
    pub(super) fn has_objects(&self) -> Option<bool> {
        (self.open && !self.chooser)
            .then(|| self.active.as_ref().is_some_and(|a| !a.hits.is_empty()))
    }
    fn index(&mut self, document: &Document, id: FootnoteId) {
        let Some(note) = document.footnote(id) else {
            return;
        };
        if let Some(old) = self.positions.remove(&id) {
            self.entries.remove(&old);
        }
        if note.number.is_none() && note.definition.is_none() {
            if self.selected == Some(id) {
                self.selected = None;
            }
            return;
        }
        let order = (note.number.is_none(), note.number.unwrap_or(0), id.0);
        self.positions.insert(id, order);
        self.entries.insert(order, id);
    }
    fn seed(&mut self, document: &Document, budget: &mut Budget) -> bool {
        while self.seeded < document.footnotes().len() {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            self.index(document, FootnoteId(self.seeded));
            self.seeded += 1;
        }
        if self.selected.is_none() {
            self.selected = self.entries.first_key_value().map(|(_, &id)| id);
        }
        true
    }
    pub(super) fn apply(&mut self, document: &Document, update: &Update) -> Result<(), FrameError> {
        if update.document != Some(document.id()) {
            return Err(LayoutError::WrongDocument.into());
        }
        if update.revision != document.revision()
            || self
                .revision
                .is_some_and(|r| r != update.base_revision && r != update.revision)
        {
            return Err(LayoutError::OutOfOrderUpdate.into());
        }
        if self.revision == Some(update.revision) {
            return Ok(());
        }
        // Preflight the hosted arenas before changing any occurrence index.
        for delta in &update.body_updates {
            let body = document
                .footnote_body(delta.body().symbol())
                .ok_or(LayoutError::InvalidRange)?;
            body.with_renderer(delta.body(), document.revision(), |native| {
                if delta.facts().value.document != Some(native.id()) {
                    return Err(LayoutError::WrongDocument);
                }
                if delta.facts().value.revision != body.parser_revision() {
                    return Err(LayoutError::OutOfOrderUpdate);
                }
                Ok(())
            })??;
        }
        if let Some(active) = &self.active
            && !active.retiring
        {
            let body = document
                .footnote_body(active.key.symbol())
                .ok_or(LayoutError::InvalidRange)?;
            active.clock.validate(body, update)?;
        }
        self.notice = None;
        if let Some(cold) = &mut self.cold {
            cold.mapping = None;
        }
        self.work.input_events += update.display_shape.events().len()
            + update.footnotes_changed.len()
            + update.new_footnotes.len();
        if let Some(pending) = &mut self.pending_move {
            pending.prepared = false;
        }
        for &id in &update.footnotes_changed {
            self.index(document, id);
        }
        for i in update.new_footnotes.clone() {
            self.index(document, FootnoteId(i));
        }
        if update.new_footnotes.start <= self.seeded {
            self.seeded = self.seeded.max(update.new_footnotes.end);
        }
        for effect in update.display_shape.events() {
            if let mdtui3::DisplayShapeEffectKind::FootnoteReference {
                id,
                reference,
                state,
            } = effect.kind
            {
                let key = OccurrenceKey::Root(document.id(), id);
                self.remove_reference(key);
                if state.is_some() {
                    self.insert_reference(
                        (reference.symbol.0, reference.source.start),
                        Occurrence {
                            key,
                            address: ReturnAddress::Root(document.id(), reference.source.start),
                        },
                    );
                }
            }
        }
        for delta in &update.body_updates {
            self.work.input_events += delta.facts().value.display_shape.events().len();
            let body = document
                .footnote_body(delta.body().symbol())
                .ok_or(LayoutError::InvalidRange)?;
            if body.key() != delta.body() {
                return Err(LayoutError::InvalidRange.into());
            }
            for effect in delta.facts().value.display_shape.events() {
                if let mdtui3::DisplayShapeEffectKind::FootnoteReference {
                    id,
                    reference,
                    state,
                } = effect.kind
                {
                    let key = OccurrenceKey::Body(body.qualify(id));
                    self.remove_reference(key);
                    if state.is_none() {
                        continue;
                    }
                    let symbol = body
                        .canonical_footnote_symbol(body.qualify(reference.symbol))?
                        .ok_or(LayoutError::InvalidRange)?;
                    let source = BodySpan {
                        body: body.key(),
                        span: reference.source,
                    };
                    let mut map = body.map(mdtui3::BodyAddress::Projected(source))?;
                    let mut work = Budget::new(WorkLimit::unlimited());
                    let mut physical = None;
                    loop {
                        match map.advance(body, &mut work)? {
                            mdtui3::BodyMapStep::Mapping(m) => {
                                physical =
                                    Some(physical.map_or(m.original.start, |p: usize| {
                                        p.min(m.original.start)
                                    }));
                            }
                            mdtui3::BodyMapStep::Complete => break,
                            _ => unreachable!(),
                        }
                    }
                    let at = physical.ok_or(LayoutError::InvalidRange)?;
                    add_work(
                        &mut self.work.input_mapping,
                        WorkLimit::default(),
                        work.used(),
                    );
                    self.insert_reference(
                        (symbol.0, at),
                        Occurrence {
                            key,
                            address: ReturnAddress::Body(source),
                        },
                    );
                }
            }
        }
        if let Some(active) = &mut self.active {
            let body = document
                .footnote_body(active.key.symbol())
                .ok_or(LayoutError::InvalidRange)?;
            active.apply(body, update)?;
        }
        self.revision = Some(update.revision);
        Ok(())
    }
    fn remove_reference(&mut self, key: OccurrenceKey) {
        if let Some(position) = self.reference_positions.remove(&key) {
            self.references.remove(&position);
        }
    }
    fn insert_reference(&mut self, position: (usize, usize), occurrence: Occurrence) {
        self.reference_positions.insert(occurrence.key, position);
        self.references.insert(position, occurrence);
    }
    pub(super) fn drain_cancelled<W: Write + ?Sized>(
        &mut self,
        document: &Document,
        root_links: &super::super::objects::SharedLinks,
        tables: &mut super::super::table_widths::TableWidthCache,
        writer: &mut W,
        budget: &mut Budget,
        wire_pending: bool,
    ) -> Result<bool, FrameError> {
        if wire_pending
            || self.open
                && !self.chooser
                && self
                    .active
                    .as_ref()
                    .is_none_or(|a| !a.retiring && self.selected == Some(a.key.symbol()))
        {
            return Ok(true);
        }
        let Some(active) = &mut self.active else {
            return Ok(true);
        };
        active.reading.cancel(&active.viewport.shared_links());
        active.raw_copy.cancel(&active.viewport.shared_links());
        active.raw_map = None;
        if active.raw_copy.copy_pending() {
            active
                .raw_copy
                .advance(document, root_links, budget, writer, tables);
        }
        if active.reading.copy_pending() {
            let body = document
                .footnote_body(active.key.symbol())
                .ok_or(LayoutError::InvalidRange)?;
            body.with_renderer(active.key, body.host_revision(), |native| {
                let links = active.viewport.shared_links();
                let (owner, tables) = active.viewport.reading_parts_mut();
                active.reading.advance_with_view(
                    native,
                    &links,
                    budget,
                    writer,
                    tables,
                    Some(owner),
                );
            })?;
        }
        Ok(!active.raw_copy.copy_pending() && !active.reading.copy_pending())
    }
    fn choose(&mut self, forward: bool) {
        let order = self
            .selected
            .and_then(|id| self.positions.get(&id).copied());
        self.selected = if forward {
            order.and_then(|key| {
                self.entries
                    .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
                    .next()
                    .map(|(_, &id)| id)
            })
        } else {
            order.and_then(|key| self.entries.range(..key).next_back().map(|(_, &id)| id))
        }
        .or(self.selected);
    }
}

fn number(number: Option<usize>) -> String {
    number.map_or_else(
        || "—".to_owned(),
        |n| {
            n.to_string()
                .chars()
                .map(|ch| {
                    ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'][ch as usize - '0' as usize]
                })
                .collect()
        },
    )
}
fn target(document: &Document, id: FootnoteId) -> &'static str {
    if document
        .footnote(id)
        .is_some_and(|n| n.definition.is_some())
    {
        "Defined"
    } else if document.is_finished() {
        "Missing"
    } else {
        "Unresolved"
    }
}
fn ui_line(text: &str, width: u16) -> super::super::sink::SinkLine {
    let mut cells = vec![];
    let mut used = 0;
    for glyph in graphemes(text) {
        if used + u16::from(glyph.width) > width {
            break;
        }
        used += u16::from(glyph.width);
        cells.push(super::super::sink::SinkCell {
            text: glyph.text.into(),
            width: glyph.width,
            style: StyleRole::META,
            ..Default::default()
        });
    }
    super::super::sink::SinkLine {
        cells,
        source: None,
    }
}

impl FrameRuntime {
    /// Diagnostic snapshot; capacity inspection is explicit, outside drawing.
    pub fn footnote_stats(&self) -> Result<FootnoteSurfaceStats, FrameError> {
        let mut stats = self.notes.work;
        stats.saved_intents = self.notes.intents.len();
        stats.saved_intent_records = self.notes.intents.values().map(|i| i.fold.records()).sum();
        stats.source_occurrences = self.notes.references.len();
        if let Some(active) = &self.notes.active {
            let body = self
                .document
                .document()
                .footnote_body(active.key.symbol())
                .ok_or(LayoutError::InvalidRange)?;
            body.with_renderer(active.key, body.host_revision(), |_| {
                stats.active_owner = Some(active.key);
                stats.shape = active.shape.work();
                stats.objects = active.viewport.shared_links().stats();
                stats.height_capacity_bytes =
                    active.viewport.heights().arena_stats().capacity_bytes
                        + active
                            .viewport
                            .heights()
                            .dependency_arena_stats()
                            .capacity_bytes;
                stats.checkpoint_capacity_bytes =
                    active.viewport.checkpoints().arena_stats().capacity_bytes;
                stats.table_capacity_bytes = active
                    .viewport
                    .table_widths()
                    .memory_stats()
                    .tracked_capacity_bytes;
                stats.math_owners = active.viewport.shared_links().math().retained_owners();
                stats.math_retirement_jobs =
                    active.viewport.shared_links().math().retirement_jobs();
            })?;
        }
        Ok(stats)
    }
    pub fn footnote_fold_state(
        &self,
        id: BodyId<mdtui3::FoldId>,
        host_revision: u64,
        budget: &mut Budget,
    ) -> Result<Option<NoteFoldState>, FrameError> {
        if id.body().document() != self.document.document().id() {
            return Err(LayoutError::WrongDocument.into());
        }
        let body = self
            .document
            .document()
            .footnote_body(id.body().symbol())
            .ok_or(LayoutError::InvalidRange)?;
        body.with_renderer(id.body(), host_revision, |native| {
            let active = self
                .notes
                .active
                .as_ref()
                .ok_or(LayoutError::InvalidRange)?;
            if active.key != id.body() {
                return Err(LayoutError::InvalidRange);
            }
            if active.retiring || active.restoring.is_some() || !active.viewport.matches(native) {
                return Err(LayoutError::OutOfOrderUpdate);
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(None);
            }
            let view = active.viewport.folds();
            let Some(candidate) = view.fold_candidate(native, id.local())? else {
                return Ok(None);
            };
            Ok(Some(NoteFoldState {
                id,
                source: BodySpan {
                    body: body.key(),
                    span: candidate.source,
                },
                hidden: BodySpan {
                    body: body.key(),
                    span: candidate.hidden,
                },
                collapsed: view.is_collapsed(id.local()),
                preview: view.preview().map(|id| body.qualify(id)),
                defaults: view.defaults(),
                version: view.version(),
            }))
        })?
        .map_err(FrameError::from)
    }
    pub(crate) fn footnote_ui_open(&self) -> bool {
        self.notes.open
    }
    pub(super) fn note_directory_ready(&mut self, source: usize) -> Result<(), FrameError> {
        let position = self.navigation_position().unwrap_or(HistoryPosition {
            source,
            row_offset: 0,
        });
        self.stage_surface_move(
            SurfacePosition::Root {
                document: self.document.document().id(),
                position,
            },
            false,
            None,
            false,
        )?;
        self.notes.pending_move.as_mut().unwrap().prepared = true;
        self.notes.open = false;
        self.notes.chooser = false;
        self.body_dirty = true;
        Ok(())
    }
    pub(crate) fn take_footnote_external_target(
        &mut self,
    ) -> Option<mdtui3::BodyObject<crate::external::ExternalTarget>> {
        let (revision, target) = self.notes.external.take()?;
        if revision != self.document.document().revision() {
            return None;
        }
        let body = self
            .document
            .document()
            .footnote_body(target.body.symbol())?;
        (body.key() == target.body).then_some(target)
    }
    pub(crate) fn set_footnote_notice(&mut self, notice: &str) {
        self.notes.notice = Some(notice.to_owned());
    }
    pub fn focus_footnote_at(&mut self, row: u16, col: u16) -> bool {
        let link = self
            .presenter
            .screen()
            .front_cell(row, col)
            .and_then(|c| c.link_id());
        let Some(source) = self
            .presenter
            .screen()
            .front_cell(row, col)
            .and_then(|c| c.source())
        else {
            return false;
        };
        if self.notes.open {
            if self.notes.chooser {
                return false;
            }
            let Some(active) = &mut self.notes.active else {
                return false;
            };
            if active.retiring
                || active.restoring.is_some()
                || self.notes.screen_body != Some(active.key)
            {
                return false;
            }
            let address = BodySpan {
                body: active.key,
                span: source,
            };
            let Some(hit) = active
                .hits
                .iter()
                .find(|h| {
                    h.source() == address
                        || matches!(h,NoteObject::Link(id,_) if Some(id.local())==link)
                })
                .copied()
            else {
                return false;
            };
            active.focus = Some(hit);
            self.presenter.close_open_cut();
            return true;
        }
        let Some(id) = self.document.document().footnote_reference_at(source) else {
            return false;
        };
        self.object_focus = Some(VisibleObject::Footnote(id, source));
        self.link_focus = None;
        true
    }
    pub(super) fn footnote_focus_status(&self) -> Option<String> {
        let Some(VisibleObject::Footnote(id, _)) = self.object_focus else {
            return None;
        };
        let reference = self.document.document().footnote_references().get(id.0)?;
        let note = self.document.document().footnote(reference.symbol)?;
        Some(format!(
            "{} {} [{}] | Enter: note",
            number(note.number),
            note.label,
            target(self.document.document(), reference.symbol)
        ))
    }
    pub fn surface_position(&self) -> Option<SurfacePosition> {
        self.notes.published.or_else(|| {
            self.navigation_position()
                .map(|position| SurfacePosition::Root {
                    document: self.document.document().id(),
                    position,
                })
        })
    }
    pub fn footnote_navigation_completion(&self) -> (u64, Option<SurfaceMove>) {
        (self.notes.move_serial, self.notes.move_result)
    }
    fn stage_surface_move(
        &mut self,
        to: SurfacePosition,
        restore: bool,
        origin: Option<ReturnAddress>,
        pop_origin: bool,
    ) -> Result<(), FrameError> {
        if to.document() != self.document.document().id() {
            return Err(LayoutError::WrongDocument.into());
        }
        let from = self.surface_position().unwrap_or(SurfacePosition::Root {
            document: self.document.document().id(),
            position: HistoryPosition {
                source: 0,
                row_offset: 0,
            },
        });
        self.notes.pending_move = Some(PendingMove {
            from,
            to,
            restore,
            origin,
            pop_origin,
            root_existing: false,
            prepared: false,
        });
        Ok(())
    }
    pub fn restore_surface_position(&mut self, to: SurfacePosition) -> Result<(), FrameError> {
        if to.document() != self.document.document().id() {
            return Err(LayoutError::WrongDocument.into());
        }
        if let SurfacePosition::Body { source, .. } = to {
            let body = self
                .document
                .document()
                .footnote_body(source.body.symbol())
                .ok_or(LayoutError::InvalidRange)?;
            if source.body != body.key() {
                return Err(LayoutError::InvalidRange.into());
            }
            self.notes.selected = Some(source.body.symbol());
            self.notes.reveal = Some(source);
            self.notes.chooser = false;
            self.notes.open = true;
        }
        self.stage_surface_move(to, true, None, false)
    }
    pub(super) fn progress_surface_restore(
        &mut self,
        budget: &mut Budget,
    ) -> Result<bool, FrameError> {
        let Some(pending) = self.notes.pending_move else {
            return Ok(true);
        };
        if pending.prepared {
            return Ok(true);
        }
        let SurfacePosition::Root { document, position } = pending.to else {
            return Ok(true);
        };
        if document != self.document.document().id() {
            return Err(LayoutError::WrongDocument.into());
        }
        let ready = if pending.root_existing {
            true
        } else if pending.restore {
            match self.restore_navigation_position(position, budget)? {
                HistoryRestoreStatus::BudgetExhausted => return Ok(false),
                HistoryRestoreStatus::Ready { .. } => true,
                _ => false,
            }
        } else {
            match self.refine_source_to_top(position.source, budget)? {
                RefineJump::Pending => return Ok(false),
                RefineJump::Ready(_) => true,
                _ => false,
            }
        };
        if ready {
            self.notes.open = false;
            self.notes.chooser = false;
            self.notes.pending_move.as_mut().unwrap().prepared = true;
        } else {
            self.notes.fail_move();
            self.notes.notice = Some("Source target not found".to_owned());
        }
        Ok(true)
    }
    pub(super) fn note_root_committed(&mut self) {
        if self.notes.open {
            return;
        }
        self.notes.screen_body = None;
        let position = self.navigation_position().unwrap_or(HistoryPosition {
            source: 0,
            row_offset: 0,
        });
        self.notes.finish_move(SurfacePosition::Root {
            document: self.document.document().id(),
            position,
        });
    }
    pub fn footnote_reading(&self) -> Option<FootnoteReading<'_>> {
        let body = self.footnote_surface()?;
        let active = self.notes.active.as_ref()?;
        Some(FootnoteReading {
            body,
            tools: &active.reading,
            status: self.notes.reading()?,
        })
    }
    pub(super) fn note_focus(&mut self, forward: bool, cycle: bool) -> Option<bool> {
        if !self.notes.open || self.notes.chooser {
            return None;
        }
        let active = self.notes.active.as_mut()?;
        if active.retiring || active.restoring.is_some() {
            return Some(false);
        }
        active.paint = None;
        if active.hits.is_empty() {
            return Some(false);
        }
        let at = cycle
            .then(|| active.hits.iter().position(|&h| Some(h) == active.focus))
            .flatten();
        let next = at.map_or(if forward { 0 } else { active.hits.len() - 1 }, |i| {
            if forward {
                (i + 1) % active.hits.len()
            } else {
                (i + active.hits.len() - 1) % active.hits.len()
            }
        });
        let changed = active.focus != Some(active.hits[next]);
        active.focus = Some(active.hits[next]);
        self.notes.notice = None;
        Some(changed)
    }
    pub fn footnote_surface(&self) -> Option<BodyKey> {
        if !self.notes.open || self.notes.chooser {
            return None;
        }
        let key = self.notes.screen_body?;
        self.notes
            .active
            .as_ref()
            .is_some_and(|v| !v.retiring && v.key == key)
            .then_some(key)
    }
    pub fn footnote_chooser_open(&self) -> bool {
        self.notes.open && self.notes.chooser
    }
    pub fn footnote_source_at(&self, row: u16, col: u16) -> Option<BodySpan> {
        let body = self.footnote_surface()?;
        Some(BodySpan {
            body,
            span: self.screen().front_cell(row, col)?.source()?,
        })
    }
    pub fn footnote_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        body_focus: bool,
    ) -> Result<bool, FrameError> {
        let plain = !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if modifiers.contains(KeyModifiers::ALT) && matches!(code, KeyCode::Left | KeyCode::Right) {
            return Ok(false);
        }
        if !self.notes.open {
            if self.reading.input().is_some() || self.fold_ui.active() {
                return Ok(false);
            }
            if code == KeyCode::Char('f') && plain && body_focus {
                self.notes.backlinks = None;
                self.notes.selected_reference = None;
                self.reading.cancel(&self.viewport.shared_links());
                self.presenter.reset_baseline()?;
                self.notes.open = true;
                self.notes.chooser = true;
                self.pause_following();
                return Ok(true);
            }
            if code == KeyCode::Enter
                && body_focus
                && let Some(VisibleObject::Footnote(id, source)) = self.object_focus
            {
                let mut budget = Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    dependencies: 1,
                    ..Default::default()
                });
                let Some(resolution) = self
                    .document
                    .document()
                    .resolve_footnote(id, &mut budget)
                    .map_err(|_| LayoutError::InvalidRange)?
                else {
                    self.notice = Some("Footnote reference is no longer active");
                    return Ok(true);
                };
                if matches!(resolution.target, FootnoteTarget::Defined(_)) {
                    self.reading.cancel(&self.viewport.shared_links());
                    let body = self
                        .document
                        .document()
                        .footnote_body(resolution.symbol)
                        .ok_or(LayoutError::InvalidRange)?;
                    let target = BodySpan {
                        body: body.key(),
                        span: mdtui3::Span::new(0, 0),
                    };
                    self.stage_surface_move(
                        SurfacePosition::Body {
                            source: target,
                            row_offset: 0,
                        },
                        false,
                        Some(ReturnAddress::Root(
                            self.document.document().id(),
                            source.start,
                        )),
                        false,
                    )?;
                    self.notes.reveal = Some(target);
                    self.notes.selected = Some(resolution.symbol);
                    self.notes.chooser = false;
                    self.notes.open = true;
                    self.pause_following();
                } else {
                    self.notice = Some(if matches!(resolution.target, FootnoteTarget::Missing) {
                        "Footnote missing"
                    } else {
                        "Footnote waiting for definition"
                    });
                }
                return Ok(true);
            }
            return Ok(false);
        }
        if !body_focus {
            return Ok(false);
        }
        if let Some(active) = &mut self.notes.active {
            active.paint = None;
        }
        // Explicit view controls must finish the old source-tail cut before
        // their changed notice/selection can be acknowledged. Input append
        // keeps the existing continuation path and does not pass here.
        self.presenter.close_open_cut();
        if self
            .notes
            .active
            .as_ref()
            .is_some_and(|a| a.retiring || a.restoring.is_some())
            && !matches!(code, KeyCode::Esc | KeyCode::Char('b' | 'f' | 'q' | 'c'))
        {
            return Ok(true);
        }
        if let Some(active) = &mut self.notes.active
            && !self.notes.chooser
        {
            let body = self
                .document
                .document()
                .footnote_body(active.key.symbol())
                .ok_or(LayoutError::InvalidRange)?;
            let links = active.viewport.shared_links();
            if body.with_renderer(active.key, body.host_revision(), |native| {
                active.reading.input_key(code, modifiers, native, &links)
            })? {
                return Ok(true);
            }
            if code == KeyCode::Esc
                && (active.reading.selection.active()
                    || !active.reading.query().is_empty()
                    || active.reading.copy_pending()
                    || active.raw_copy.copy_pending()
                    || active.raw_map.is_some())
            {
                active.reading.cancel(&links);
                active.raw_copy.cancel(&links);
                active.raw_map = None;
                active.raw_requested = false;
                return Ok(true);
            }
            if active.fold_ui.key(code) {
                return Ok(true);
            }
            if matches!(code, KeyCode::Tab | KeyCode::BackTab) {
                return Ok(false);
            }
            if code == KeyCode::Enter
                && let Some(NoteObject::Footnote(id, source)) = active.focus
            {
                let mut budget = Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    dependencies: 1,
                    ..Default::default()
                });
                if let mdtui3::BodyResolveStep::Resolved(Some(resolved)) =
                    body.resolve_footnote(id, &mut budget)?
                {
                    if matches!(resolved.target, FootnoteTarget::Defined(_)) {
                        let next = self
                            .document
                            .document()
                            .footnote_body(resolved.symbol)
                            .ok_or(LayoutError::InvalidRange)?;
                        let destination = BodySpan {
                            body: next.key(),
                            span: mdtui3::Span::new(0, 0),
                        };
                        self.stage_surface_move(
                            SurfacePosition::Body {
                                source: destination,
                                row_offset: 0,
                            },
                            false,
                            Some(ReturnAddress::Body(source)),
                            false,
                        )?;
                        self.notes.selected = Some(resolved.symbol);
                        self.notes.reveal = Some(destination);
                    } else {
                        self.notes.notice = Some(format!(
                            "Footnote {}: {}",
                            resolved.number,
                            target(self.document.document(), resolved.symbol)
                        ));
                    }
                }
                return Ok(true);
            }
            if code == KeyCode::Enter
                && let Some(NoteObject::Fold(id, _)) = active.focus
            {
                if id.body() != active.key {
                    return Err(LayoutError::InvalidRange.into());
                }
                if active.focus_fold_version != active.viewport.folds().version() {
                    self.notes.notice = Some("Fold focus is stale; Tab to select".to_owned());
                    return Ok(true);
                }
                active.fold_ui.activate(id.local());
                return Ok(true);
            }
            if code == KeyCode::Enter
                && let Some(NoteObject::Link(id, _)) = active.focus
            {
                if id.body() != active.key {
                    return Err(LayoutError::InvalidRange.into());
                }
                active.queued_link = Some(id);
                if let Some(link) = &mut active.link {
                    link.cancelled = true;
                }
                return Ok(true);
            }
        }
        if matches!(code, KeyCode::Char('q'))
            || modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c')
        {
            return Ok(false);
        }
        if code == KeyCode::Char('b') && plain || code == KeyCode::Esc {
            if self.notes.pending_move.is_some() {
                self.notes.fail_move();
                match self.notes.published {
                    Some(position @ SurfacePosition::Body { .. }) => {
                        self.restore_surface_position(position)?
                    }
                    _ => {
                        self.notes.open = false;
                        self.notes.chooser = false;
                        self.presenter.reset_baseline()?;
                        self.body_dirty = true;
                    }
                }
                return Ok(true);
            }
            if code == KeyCode::Char('b')
                && self.notes.origins.is_empty()
                && !self.notes.chooser
                && let Some(active) = &self.notes.active
            {
                self.notes.backlinks = Some(active.key.symbol());
                self.notes.selected_reference = None;
                self.notes.chooser = true;
                self.presenter.reset_baseline()?;
                return Ok(true);
            }
            if let Some(active) = &mut self.notes.active {
                let links = active.viewport.shared_links();
                active.reading.cancel(&links);
                active.raw_copy.cancel(&links);
                active.raw_map = None;
            }
            if self.notes.pending_move.is_some() {
                self.notes.fail_move();
            }
            match self.notes.origins.last().copied() {
                Some(ReturnAddress::Root(document, source)) => {
                    if document != self.document.document().id() {
                        return Err(LayoutError::WrongDocument.into());
                    }
                    self.stage_surface_move(
                        SurfacePosition::Root {
                            document,
                            position: HistoryPosition {
                                source,
                                row_offset: 0,
                            },
                        },
                        false,
                        None,
                        true,
                    )?;
                }
                Some(ReturnAddress::Body(source)) => {
                    if source.body.document() != self.document.document().id() {
                        return Err(LayoutError::WrongDocument.into());
                    }
                    self.notes.selected = Some(source.body.symbol());
                    self.notes.reveal = Some(source);
                    self.notes.open = true;
                    self.stage_surface_move(
                        SurfacePosition::Body {
                            source,
                            row_offset: 0,
                        },
                        false,
                        None,
                        true,
                    )?;
                }
                None => {
                    let position = self.navigation_position().unwrap_or(HistoryPosition {
                        source: 0,
                        row_offset: 0,
                    });
                    self.stage_surface_move(
                        SurfacePosition::Root {
                            document: self.document.document().id(),
                            position,
                        },
                        false,
                        None,
                        false,
                    )?;
                    self.notes.pending_move.as_mut().unwrap().root_existing = true;
                }
            }
            return Ok(true);
        }
        if code == KeyCode::Char('f') && plain {
            self.notes.backlinks = None;
            self.notes.chooser = true;
            self.presenter.reset_baseline()?;
            return Ok(true);
        }
        if self.notes.chooser {
            if matches!(code, KeyCode::Left | KeyCode::Right) {
                self.notes.chooser_offset = if code == KeyCode::Left {
                    self.notes.chooser_offset.saturating_sub(1)
                } else {
                    self.notes.chooser_offset.saturating_add(1).min(1200)
                };
                return Ok(true);
            }
            self.notes.screen_body = None;
            if let Some(symbol) = self.notes.backlinks {
                let range = (symbol.0, 0)..=(symbol.0, usize::MAX);
                let current = self.notes.selected_reference.or_else(|| {
                    self.notes
                        .references
                        .range(range.clone())
                        .next()
                        .map(|(&p, _)| p)
                });
                match code {
                    KeyCode::Down | KeyCode::PageDown => {
                        self.notes.selected_reference = current
                            .and_then(|p| {
                                self.notes
                                    .references
                                    .range((
                                        std::ops::Bound::Excluded(p),
                                        std::ops::Bound::Included((symbol.0, usize::MAX)),
                                    ))
                                    .next()
                                    .map(|(&p, _)| p)
                            })
                            .or(current)
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        self.notes.selected_reference = current
                            .and_then(|p| {
                                self.notes
                                    .references
                                    .range((
                                        std::ops::Bound::Included((symbol.0, 0)),
                                        std::ops::Bound::Excluded(p),
                                    ))
                                    .next_back()
                                    .map(|(&p, _)| p)
                            })
                            .or(current)
                    }
                    KeyCode::Home => {
                        self.notes.selected_reference = self
                            .notes
                            .references
                            .range(range.clone())
                            .next()
                            .map(|(&p, _)| p)
                    }
                    KeyCode::End => {
                        self.notes.selected_reference = self
                            .notes
                            .references
                            .range(range)
                            .next_back()
                            .map(|(&p, _)| p)
                    }
                    KeyCode::Enter => {
                        if let Some(occurrence) =
                            current.and_then(|p| self.notes.references.get(&p)).copied()
                        {
                            match occurrence.address {
                                ReturnAddress::Root(document, source) => {
                                    if document != self.document.document().id() {
                                        return Err(LayoutError::WrongDocument.into());
                                    }
                                    self.stage_surface_move(
                                        SurfacePosition::Root {
                                            document,
                                            position: HistoryPosition {
                                                source,
                                                row_offset: 0,
                                            },
                                        },
                                        false,
                                        None,
                                        false,
                                    )?;
                                }
                                ReturnAddress::Body(source) => {
                                    self.notes.selected = Some(source.body.symbol());
                                    self.notes.reveal = Some(source);
                                    self.notes.chooser = false;
                                    self.stage_surface_move(
                                        SurfacePosition::Body {
                                            source,
                                            row_offset: 0,
                                        },
                                        false,
                                        None,
                                        false,
                                    )?;
                                }
                            }
                        }
                    }
                    _ => return Ok(false),
                }
                return Ok(true);
            }
            match code {
                KeyCode::Down | KeyCode::PageDown => self.notes.choose(true),
                KeyCode::Up | KeyCode::PageUp => self.notes.choose(false),
                KeyCode::Home => {
                    self.notes.selected = self.notes.entries.first_key_value().map(|(_, &id)| id)
                }
                KeyCode::End => {
                    self.notes.selected = self.notes.entries.last_key_value().map(|(_, &id)| id)
                }
                KeyCode::Enter => {
                    if let Some(id) = self.notes.selected {
                        if let Some(body) = self.document.document().footnote_body(id) {
                            let source = BodySpan {
                                body: body.key(),
                                span: mdtui3::Span::new(0, 0),
                            };
                            self.notes.reveal = Some(source);
                            self.notes.chooser = false;
                            self.stage_surface_move(
                                SurfacePosition::Body {
                                    source,
                                    row_offset: 0,
                                },
                                false,
                                None,
                                false,
                            )?;
                        } else {
                            self.notes.notice =
                                Some(target(self.document.document(), id).to_owned());
                        }
                    }
                }
                _ => return Ok(false),
            }
            return Ok(true);
        }
        let Some(active) = &mut self.notes.active else {
            return Ok(true);
        };
        let view = self
            .document
            .document()
            .footnote_body(active.key.symbol())
            .ok_or(LayoutError::InvalidRange)?;
        view.with_renderer(
            active.key,
            view.host_revision(),
            |native| -> Result<bool, FrameError> {
                let links = active.viewport.shared_links();
                active.reading.set_width(self.geometry.content_width);
                match code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                        if active.reading.selection.active()
                            && !modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        use crate::reading::CursorMove;
                        let direction = match code {
                            KeyCode::Left => CursorMove::Left,
                            KeyCode::Right => CursorMove::Right,
                            KeyCode::Up => CursorMove::Up,
                            _ => CursorMove::Down,
                        };
                        active.viewport.pause_following();
                        if let Some(edge) = active.reading.selection.move_cursor(
                            self.presenter.screen(),
                            ContentArea::new(self.content_x, self.geometry.content_width),
                            direction,
                            modifiers.contains(KeyModifiers::SHIFT),
                        ) {
                            let moved = if matches!(edge, CursorMove::Left | CursorMove::Up) {
                                active.viewport.scroll_up(native, 1)?
                            } else {
                                active.viewport.scroll_down(native, 1)?
                            };
                            if !moved {
                                active.reading.selection.cancel_motion();
                            }
                        }
                    }
                    KeyCode::Char('z') if plain => {
                        active.viewport.pause_following();
                        let source = active
                            .reading
                            .selection
                            .cursor()
                            .map(|s| s.start)
                            .or_else(|| active.viewport.top_source(native))
                            .unwrap_or(0);
                        let target = match active.focus {
                            Some(NoteObject::Fold(id, _)) => Some(id.local()),
                            _ => None,
                        };
                        active
                            .fold_ui
                            .open(source, active.reading.selection.range(), target);
                    }
                    KeyCode::Down | KeyCode::PageDown => {
                        active.viewport.cancel_end();
                        active.viewport.scroll_down(
                            native,
                            if code == KeyCode::Down {
                                1
                            } else {
                                u64::from(self.geometry.rows.saturating_sub(1).max(1))
                            },
                        )?;
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        active.viewport.cancel_end();
                        active.viewport.scroll_up(
                            native,
                            if code == KeyCode::Up {
                                1
                            } else {
                                u64::from(self.geometry.rows.saturating_sub(1).max(1))
                            },
                        )?;
                    }
                    KeyCode::End => {
                        active
                            .viewport
                            .seek_end(native, !modifiers.contains(KeyModifiers::CONTROL))?;
                    }
                    KeyCode::Home => {
                        active.viewport.cancel_end();
                        active.viewport.scroll_up(native, u64::MAX)?;
                    }
                    KeyCode::Char('/') if plain => active.reading.start_input(),
                    KeyCode::Char('n' | 'N') if plain => {
                        active
                            .reading
                            .navigate(native, &links, code == KeyCode::Char('N'))
                    }
                    KeyCode::Char('v') if plain => {
                        active.reading.selection.toggle(
                            self.presenter.screen(),
                            ContentArea::new(self.content_x, self.geometry.content_width),
                        );
                        active.viewport.pause_following();
                    }
                    KeyCode::Char(' ') if active.reading.selection.active() => {
                        active.reading.selection.toggle_extension()
                    }
                    KeyCode::Char('Y') if active.reading.selection.active() => {
                        active.raw_copy.cancel(&links);
                        active.raw_range = None;
                        active.raw_requested = true;
                        active.raw_map = active
                            .reading
                            .selection
                            .range()
                            .map(|span| {
                                view.map(mdtui3::BodyAddress::Projected(BodySpan {
                                    body: active.key,
                                    span,
                                }))
                            })
                            .transpose()?;
                    }
                    KeyCode::Char('y') if active.reading.selection.active() => {
                        active.raw_copy.cancel(&links);
                        active.raw_map = None;
                        active.raw_requested = false;
                        active.reading.start_copy(
                            native,
                            crate::reading::CopyKind::Visible,
                            self.geometry.content_width,
                        );
                        active.reading.bind_copy_view(
                            active.viewport.folds().version(),
                            active.viewport.folds().has_collapsed(),
                        );
                    }
                    KeyCode::Char('s') => {
                        active.viewport.pause_following();
                        return Ok(false);
                    }
                    KeyCode::Left | KeyCode::Right => {
                        if !active.viewport.checkpoints().maintenance_in_progress()
                            && let Some(status) = active.viewport.shown_tables.first().copied()
                        {
                            let delta = if modifiers.contains(KeyModifiers::CONTROL) {
                                8
                            } else {
                                1
                            };
                            let delta = if code == KeyCode::Left { -delta } else { delta };
                            let first = active
                                .viewport
                                .table_widths()
                                .first_column(status.table_id)
                                .saturating_add_signed(delta);
                            if active.viewport.set_table_first_column(
                                native,
                                status.table_id,
                                first,
                            )? {
                                active.reading.view_changed();
                            }
                        }
                    }
                    _ => return Ok(true),
                }
                Ok(true)
            },
        )?
    }
    pub(super) fn frame_footnotes<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        budget: &mut Budget,
    ) -> Result<FrameCommit, FrameError> {
        self.ensure_sidebar_model()?;
        self.prepare_sidebar(budget)?;
        self.advance_reading(writer, budget);
        let document = self.document.document();
        if !self.notes.seed(document, budget) {
            return Ok(FrameCommit {
                status: Some(ViewportFrameStatus::BudgetExhausted),
                ..Default::default()
            });
        }
        let rows = self.geometry.rows.saturating_sub(1).max(1);
        if self.notes.chooser {
            self.notes.screen_body = None;
            let Some(frame) =
                self.notes
                    .chooser_frame(document, self.geometry.content_width, rows, budget)?
            else {
                return Ok(FrameCommit::default());
            };
            let help = if self.notes.backlinks.is_some() {
                "Enter: source | f: notes | Esc: main"
            } else {
                "Enter: open | arrows: choose/label | b: return"
            };
            return self.presenter.commit(
                writer,
                document,
                &frame,
                FramePaintLayout {
                    content_x: self.content_x,
                    content_width: self.geometry.content_width,
                    sidebar_width: self.sidebar_width,
                    sidebar: self.sidebar.as_ref(),
                    focused_link: None,
                    notice: self.notes.notice.as_deref().or(Some(help)),
                    following: false,
                    table_notice: false,
                    selection: None,
                    cursor: None,
                    search: None,
                },
                Instant::now(),
                budget,
            );
        }
        let Some(id) = self.notes.selected else {
            self.notes.chooser = true;
            return Ok(FrameCommit::default());
        };
        let body = document
            .footnote_body(id)
            .ok_or(LayoutError::InvalidRange)?;
        if self
            .notes
            .active
            .as_ref()
            .is_none_or(|a| a.retiring || a.key != body.key())
        {
            if self.notes.active.is_some() {
                if let Some(tx) = self.presenter.wire.as_mut() {
                    tx.cancel();
                    if tx.advance(writer, budget)? != Progress::Complete {
                        return Ok(FrameCommit::default());
                    }
                    self.presenter.wire = None;
                    self.presenter.sent = None;
                    self.presenter.full_repaint = true;
                }
                if !self.notes.active.as_ref().unwrap().retiring {
                    self.presenter.reset_baseline()?;
                    // ACK snapshots also retain mathematical payload leases.
                    // Release the old surface before its paid retirement.
                    self.presenter.sent = None;
                }
                if !self.notes.drain_cancelled(
                    document,
                    &self.viewport.shared_links(),
                    self.viewport.table_widths_mut(),
                    writer,
                    budget,
                    false,
                )? {
                    return Ok(FrameCommit::default());
                }
                let old_key = self.notes.active.as_ref().unwrap().key;
                let old_body = document
                    .footnote_body(old_key.symbol())
                    .ok_or(LayoutError::InvalidRange)?;
                let before = budget.used();
                let completion = self
                    .notes
                    .active
                    .as_mut()
                    .unwrap()
                    .retire(old_body, budget)?;
                add_work(&mut self.notes.work.retirement, before, budget.used());
                if completion != Completion::Covered {
                    return Ok(FrameCommit::default());
                }
                let mut old = self.notes.active.take().unwrap();
                let fold = old.saved_fold.take().ok_or(LayoutError::InvalidRange)?;
                if !fold.is_empty() || old.fold_ui.has_policies() {
                    self.notes.intents.insert(
                        old.key,
                        NoteIntent {
                            fold,
                            ui: std::mem::take(&mut old.fold_ui),
                        },
                    );
                }
                self.notes.work.note_retirements += 1;
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(FrameCommit::default());
            }
            self.notes.active = Some(Box::new(NoteViewport::new(
                body,
                self.geometry.content_width,
                rows,
            )?));
            self.notes.work.note_creations += 1;
            if let Some(intent) = self.notes.intents.remove(&body.key()) {
                let active = self.notes.active.as_mut().unwrap();
                active.restoring = Some(intent.fold);
                active.fold_ui = intent.ui;
            }
            self.presenter.reset_baseline()?;
        }
        let mut finished_position = None;
        let mut failed_move = false;
        let mut link_result = NoteLinkResult::Done;
        let mut published_body = false;
        let active = self.notes.active.as_mut().unwrap();
        if !active.restore_choices(body, budget)? {
            return Ok(FrameCommit::default());
        }
        if let Some(source) = self.notes.reveal.take() {
            if source.body != active.key {
                return Err(LayoutError::InvalidRange.into());
            }
            active.pending_target = Some(source);
            active.refiner = None;
            active.restore_offset =
                self.notes
                    .pending_move
                    .filter(|p| p.restore)
                    .and_then(|p| match p.to {
                        SurfacePosition::Body { row_offset, .. } => Some(row_offset),
                        _ => None,
                    });
        }
        let result = body.with_renderer(
            active.key,
            body.host_revision(),
            |native| -> Result<FrameCommit, FrameError> {
                if active.viewport.config().width != self.geometry.content_width {
                    active
                        .viewport
                        .set_content_width(native, self.geometry.content_width)?;
                    active.reading.width_changed();
                    active.refiner = None;
                    active.reveal_task = None;
                    active.revealed = None;
                }
                if active.viewport.config().rows != rows {
                    active.viewport.set_viewport_rows(rows)?;
                }
                if !active.viewport.initialize(native, budget)? {
                    return Ok(FrameCommit::default());
                }
                if active.viewport.checkpoints().maintenance_in_progress()
                    && active
                        .viewport
                        .maintain_checkpoints(native, budget)?
                        .completion
                        == Completion::BudgetExhausted
                {
                    return Ok(FrameCommit::default());
                }
                let links = active.viewport.shared_links();
                link_result = active.advance_link(body, native, budget)?;
                let clipboard_before =
                    active.reading.clipboard_bytes() + active.raw_copy.clipboard_bytes();
                if let Some(map) = active.raw_map.as_mut() {
                    loop {
                        match map.advance(body, budget)? {
                            mdtui3::BodyMapStep::BudgetExhausted => {
                                return Ok(FrameCommit::default());
                            }
                            mdtui3::BodyMapStep::Mapping(mapping) => {
                                active.raw_range =
                                    Some(active.raw_range.map_or(mapping.original, |old| {
                                        mdtui3::Span::new(
                                            old.start.min(mapping.original.start),
                                            old.end.max(mapping.original.end),
                                        )
                                    }));
                            }
                            mdtui3::BodyMapStep::Complete => break,
                        }
                    }
                    active.raw_map = None;
                    if let Some(range) = active.raw_range.take() {
                        if active.key.document() != document.id() {
                            return Err(LayoutError::WrongDocument.into());
                        }
                        active.raw_copy.start_source_range(
                            document,
                            range,
                            self.geometry.content_width,
                        );
                    }
                }
                if !self.presenter.wire_pending() && active.raw_copy.copy_pending() {
                    active.raw_copy.advance(
                        document,
                        &self.viewport.shared_links(),
                        budget,
                        writer,
                        self.viewport.table_widths_mut(),
                    );
                }
                active.fold_ui.advance(
                    native,
                    active.viewport.folds_mut(),
                    &links,
                    self.geometry.content_width,
                    budget,
                );
                if active.fold_ui.take_changed() {
                    active.viewport.fold_changed(native);
                    active.reading.view_changed();
                    active.refiner = None;
                }
                if let Some(summary) = active.fold_ui.take_summary() {
                    active.viewport.set_fold_summary(native, summary)?;
                    active.reading.view_changed();
                    active.refiner = None;
                }
                if active.fold_ui.prefix_validation_pending() {
                    return Ok(FrameCommit::default());
                }
                let (owner, tables) = active.viewport.reading_parts_mut();
                if !self.presenter.wire_pending()
                    && let Some(source) = active.reading.advance_with_view(
                        native,
                        &links,
                        budget,
                        writer,
                        tables,
                        Some(owner),
                    )
                {
                    active.pending_target = Some(BodySpan {
                        body: active.key,
                        span: mdtui3::Span::new(source, source),
                    });
                    active.refiner = None;
                }
                if let Some(source) = active.pending_target {
                    active.viewport.cancel_end();
                    if source.body != active.key {
                        return Err(LayoutError::InvalidRange.into());
                    }
                    if source.span.start == 0 && active.restore_offset.is_none_or(|o| o == 0) {
                        active.viewport.cancel_end();
                        active.viewport.scroll_up(native, u64::MAX)?;
                        active.pending_target = None;
                        active.restore_offset = None;
                        active.refiner = None;
                    }
                }
                if let Some(source) = active.pending_target {
                    let signature = (
                        native.revision(),
                        active.viewport.folds().version(),
                        source.span.start,
                    );
                    if active.revealed != Some(signature) {
                        if active.reveal_task.is_none() {
                            active.reveal_task = Some(mdtui3::FoldNavigation::new(
                                native,
                                active.viewport.folds(),
                                &active.navigator,
                                NavAction::Source(source.span.start),
                                mdtui3::FoldNavigationPolicy::Reveal,
                                u64::from(rows),
                            )?);
                        }
                        let progress = active.reveal_task.as_mut().unwrap().advance(
                            native,
                            active.viewport.folds_mut(),
                            budget,
                        )?;
                        if progress.change.is_some() {
                            active.viewport.fold_changed(native);
                            active.reading.view_changed();
                            active.refiner = None;
                        }
                        if progress.completion == Completion::BudgetExhausted {
                            return Ok(FrameCommit::default());
                        }
                        active.reveal_task = None;
                        active.revealed = Some((
                            native.revision(),
                            active.viewport.folds().version(),
                            source.span.start,
                        ));
                    }
                    if active.refiner.is_none() && !native.source().is_empty() {
                        let owner = active.viewport.layout_owner_mut();
                        let mut refiner = SourceRefiner::new(
                            native,
                            owner.heights(),
                            owner,
                            source.span.start,
                            RefineConfig {
                                width: self.geometry.content_width,
                                ..Default::default()
                            },
                        )
                        .map_err(|_| FrameError::Viewport(ViewportError::InvalidTop))?;
                        refiner.set_links(links.clone());
                        active.refiner = Some(refiner);
                    }
                    if let Some(refiner) = &mut active.refiner {
                        match refiner
                            .advance_owned(native, active.viewport.layout_owner_mut(), budget)
                            .map_err(|_| FrameError::Viewport(ViewportError::InvalidTop))?
                        {
                            super::super::refine::RefineProgress::Ready(target) => {
                                let mut anchor = target.anchor;
                                if let Some(offset) = active.restore_offset.take() {
                                    anchor.offset = offset.min(
                                        active
                                            .viewport
                                            .heights()
                                            .height(anchor.tile)
                                            .unwrap_or(0)
                                            .saturating_sub(1),
                                    );
                                }
                                active.viewport.set_top_from_refinement(
                                    native,
                                    anchor,
                                    target.line_start,
                                    target.source,
                                    target.status == RefineStatus::Exact,
                                )?;
                                active.pending_target = None;
                                active.refiner = None;
                            }
                            super::super::refine::RefineProgress::NotFound => {
                                active.pending_target = None;
                                active.refiner = None;
                                self.notes.notice = Some(
                                    "Target not found in current refinement window".to_owned(),
                                );
                                failed_move = true;
                            }
                            _ => return Ok(FrameCommit::default()),
                        }
                    } else {
                        active.pending_target = None;
                    }
                }
                let generation = PaintGeneration {
                    body: body.key(),
                    host: body.host_revision(),
                    parser: body.parser_revision(),
                    binding: body.binding_epoch(),
                    width: self.geometry.content_width,
                    rows,
                    fold: active.viewport.folds().version(),
                    position: active.viewport.top_position(),
                };
                let frame = if active.paint.as_ref().is_some_and(|(g, _)| *g == generation) {
                    active.paint.as_ref().unwrap().1.clone()
                } else {
                    active.viewport.frame(native, budget)?
                };
                if !matches!(
                    frame.status,
                    ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
                ) {
                    return Ok(FrameCommit {
                        status: Some(frame.status),
                        ..Default::default()
                    });
                }
                active.paint = Some((
                    PaintGeneration {
                        position: active.viewport.top_position(),
                        ..generation
                    },
                    frame.clone(),
                ));
                let note = document.footnote(id).ok_or(LayoutError::InvalidRange)?;
                let focused_status = if let Some(NoteObject::Footnote(reference, _)) = active.focus
                {
                    if active.note_status.as_ref().is_none_or(|(id, revision, _)| {
                        *id != reference || *revision != body.host_revision()
                    }) && let mdtui3::BodyResolveStep::Resolved(Some(resolved)) =
                        body.resolve_footnote(reference, budget)?
                    {
                        active.note_status = Some((
                            reference,
                            body.host_revision(),
                            format!(
                                "{} [{}] | Enter: note",
                                number(Some(resolved.number)),
                                target(document, resolved.symbol)
                            ),
                        ));
                    }
                    Some(
                        active
                            .note_status
                            .as_ref()
                            .map(|(_, _, s)| s.as_str())
                            .unwrap_or("Footnote status pending"),
                    )
                } else {
                    None
                };
                let status = active
                    .fold_ui
                    .status(self.geometry.content_width)
                    .or_else(|| {
                        if active.raw_requested {
                            active.raw_copy.status()
                        } else {
                            active.reading.status()
                        }
                    })
                    .or_else(|| self.notes.notice.clone())
                    .or_else(|| focused_status.map(str::to_owned))
                    .unwrap_or_else(|| {
                        format!(
                            "{} {} [{}] | b: return | f: notes",
                            number(note.number),
                            note.label,
                            target(document, id)
                        )
                    });
                let mut commit = self.presenter.commit_body(
                    writer,
                    (native, Some(body)),
                    &frame,
                    FramePaintLayout {
                        content_x: self.content_x,
                        content_width: self.geometry.content_width,
                        sidebar_width: self.sidebar_width,
                        sidebar: self.sidebar.as_ref(),
                        focused_link: match active.focus {
                            Some(NoteObject::Link(id, _)) => Some(id.local()),
                            _ => None,
                        },
                        notice: Some(&status),
                        following: active.viewport.following(),
                        table_notice: false,
                        selection: active.reading.selection.range(),
                        cursor: active
                            .reading
                            .selection
                            .cursor()
                            .or(active.focus.map(|h| h.source().span)),
                        search: active.reading.current_match().map(|h| h.source),
                    },
                    Instant::now(),
                    budget,
                )?;
                if commit.published {
                    published_body = true;
                    active.focus_fold_version = active.viewport.folds().version();
                    active.reading.selection.reproject(
                        self.presenter.screen(),
                        ContentArea::new(self.content_x, self.geometry.content_width),
                    );
                    active.hits.clear();
                    let mut seen = HashSet::new();
                    for row in 0..self.geometry.rows.saturating_sub(1) {
                        for col in self.content_x..self.content_x + self.geometry.content_width {
                            let Some(cell) = self.presenter.screen().front_cell(row, col) else {
                                continue;
                            };
                            let Some(source) = cell.source() else {
                                continue;
                            };
                            let address = BodySpan {
                                body: active.key,
                                span: source,
                            };
                            let hit = if let Some(id) = native.footnote_reference_at(source) {
                                Some(NoteObject::Footnote(body.qualify(id), address))
                            } else if let Some(id) = cell.link_id() {
                                native.link_info(id).map(|info| {
                                    NoteObject::Link(
                                        body.qualify(id),
                                        BodySpan {
                                            body: body.key(),
                                            span: info.source,
                                        },
                                    )
                                })
                            } else {
                                active
                                    .viewport
                                    .shown_folds
                                    .iter()
                                    .find(|p| p.candidate.hidden == source)
                                    .map(|p| {
                                        NoteObject::Fold(body.qualify(p.candidate.id), address)
                                    })
                            };
                            if let Some(hit) = hit
                                && seen.insert(hit.key())
                            {
                                active.hits.push(hit);
                            }
                        }
                    }
                    if let Some(old) = active.focus {
                        active.focus = active.hits.iter().find(|h| h.key() == old.key()).copied();
                    }
                }
                let clipboard_bytes = active.reading.clipboard_bytes()
                    + active.raw_copy.clipboard_bytes()
                    - clipboard_before;
                commit.bytes += clipboard_bytes;
                commit.metrics.clipboard_bytes += clipboard_bytes;
                commit.metrics.output_bytes += clipboard_bytes;
                if commit.committed {
                    let source = match active.viewport.top_position() {
                        TopPosition::Exact {
                            line_start: Some(source),
                            ..
                        } => source,
                        _ => active.viewport.top_source(native).unwrap_or(0),
                    };
                    let position = SurfacePosition::Body {
                        source: BodySpan {
                            body: active.key,
                            span: mdtui3::Span::new(source, source),
                        },
                        row_offset: active.viewport.top_anchor().map_or(0, |a| a.offset),
                    };
                    finished_position = Some(position);
                }
                Ok(commit)
            },
        )?;
        if failed_move {
            self.notes.fail_move();
        }
        if published_body {
            self.notes.screen_body = Some(body.key());
        }
        if let Some(position) = finished_position {
            self.notes.finish_move(position);
        }
        match link_result {
            NoteLinkResult::Source(source) => {
                self.stage_surface_move(
                    SurfacePosition::Body {
                        source,
                        row_offset: 0,
                    },
                    false,
                    None,
                    false,
                )?;
                let active = self.notes.active.as_mut().unwrap();
                active.pending_target = Some(source);
                active.refiner = None;
            }
            NoteLinkResult::External(target) => {
                if self.link_mode == LinkMode::OpenExternal {
                    self.notes.external = Some((document.revision(), target));
                } else {
                    self.notes.notice = Some("External link | use --open-links to open".to_owned());
                }
            }
            NoteLinkResult::Notice(notice) => self.notes.notice = Some(notice.to_owned()),
            _ => {}
        }
        result
    }
}
