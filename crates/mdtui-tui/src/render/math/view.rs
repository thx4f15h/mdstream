//! Shared native-object preparation. A view contains only the requested cell
//! rectangle; intrinsic metrics and paged AST are independent of that rectangle.
use super::storage::Paged;
use super::{
    MathEngine, MathPaint, MathProgress, MathRetirement, MathStats, Metrics, PaintStep, Rect,
};
use crate::render::payload::TextPayload;
use mdtui3::{
    Budget, Document, DocumentId, LayoutError, MathObjectId, MathObjectMeta, Span, WorkLimit,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[derive(Clone, Debug)]
pub struct ViewCell {
    pub x: usize,
    pub y: usize,
    pub width: u8,
    pub source: Span,
    pub text: TextPayload,
}
#[derive(Debug)]
pub struct MathView {
    pub meta: MathObjectMeta,
    key: super::MathKey,
    pub metrics: Metrics,
    pub has_ink: bool,
    pub rect: Rect,
    pub source_cut: Option<usize>,
    pub cells: ViewCells,
}
/// Whole-source atom plus an intrinsic glyph position. The snapshot key binds
/// this location to the actual engine, not just an object-local version count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathFocus {
    key: super::MathKey,
    prefix: bool,
    source: Span,
    row: usize,
    column: usize,
}
impl MathFocus {
    pub fn source(self) -> Span {
        self.source
    }
    pub fn row(self) -> usize {
        self.row
    }
    pub fn column(self) -> usize {
        self.column
    }
    pub(crate) fn same_object(self, other: Self) -> bool {
        self.key == other.key && self.prefix == other.prefix
    }
}
impl MathView {
    pub fn focus_at(&self, column: usize, row: usize) -> Option<MathFocus> {
        let (_, &id) = self
            .cells
            .order
            .range((row, column, 0)..=(row, column, usize::MAX))
            .next()?;
        let cell = &self.cells.pages[id];
        Some(self.focus_cell(cell))
    }
    pub(crate) fn focus_cell(&self, cell: &ViewCell) -> MathFocus {
        MathFocus {
            key: self.key,
            prefix: self.source_cut.is_some(),
            source: cell.source,
            row: cell.y,
            column: cell.x,
        }
    }
}
/// Fixed pages, including output: emitting one cell never relocates an older
/// viewport buffer. Retirement removes one descriptor per paid step.
#[derive(Debug, Default)]
pub struct ViewCells {
    pages: Paged<ViewCell>,
    order: BTreeMap<(usize, usize, usize), usize>,
}
impl ViewCells {
    pub fn iter(&self) -> impl Iterator<Item = &ViewCell> {
        self.order.values().map(|&id| &self.pages[id])
    }
    pub fn len(&self) -> usize {
        self.pages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
    pub fn storage(&self) -> super::StorageWork {
        self.pages.stats()
    }
    pub fn index_payload_bytes(&self) -> usize {
        self.order.len() * std::mem::size_of::<((usize, usize, usize), usize)>()
    }
    pub(crate) fn next_after(
        &self,
        after: Option<(usize, usize, usize)>,
    ) -> Option<((usize, usize, usize), &ViewCell)> {
        use std::ops::Bound::{Excluded, Unbounded};
        self.order
            .range((after.map_or(Unbounded, Excluded), Unbounded))
            .next()
            .map(|(key, &id)| (*key, &self.pages[id]))
    }
    fn pop(&mut self) {
        if let Some(cell) = self.pages.last() {
            self.order.remove(&(cell.y, cell.x, self.pages.len() - 1));
            self.pages.pop();
            if self.order.is_empty() {
                self.order = BTreeMap::new();
            }
        }
    }
    fn push(&mut self, cell: ViewCell) {
        let key = (cell.y, cell.x, self.pages.len());
        let id = self.pages.push(cell);
        self.order.insert(key, id);
    }
}
struct RetiredQuery {
    paint: Option<MathPaint>,
    view: Option<MathView>,
    ready: Option<Rc<MathView>>,
}
enum Retired {
    Engine(Box<MathRetirement>),
    Query(Box<RetiredQuery>),
}
impl Retired {
    fn advance(&mut self, budget: &mut Budget) -> bool {
        match self {
            Self::Engine(job) => job.advance(budget),
            Self::Query(job) => {
                let RetiredQuery { paint, view, ready } = job.as_mut();
                if let Some(p) = paint {
                    if !p.retire_step(budget) {
                        return false;
                    }
                    *paint = None;
                }
                if view.is_none()
                    && let Some(r) = ready.take()
                {
                    match Rc::try_unwrap(r) {
                        Ok(v) => *view = Some(v),
                        Err(r) => {
                            *ready = Some(r);
                            let _ = budget.charge(WorkLimit {
                                queries: 1,
                                objects: 1,
                                ..Default::default()
                            });
                            return false;
                        }
                    }
                }
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return false;
                }
                if let Some(v) = view {
                    v.cells.pop();
                    if !v.cells.is_empty() {
                        return false;
                    }
                }
                *view = None;
                true
            }
        }
    }
}
fn retire_one(jobs: &mut BTreeMap<usize, Retired>, serial: &mut usize, budget: &mut Budget) {
    if jobs.is_empty() || budget.remaining().queries == 0 || budget.remaining().objects == 0 {
        return;
    }
    let (id, mut job) = jobs.pop_first().unwrap();
    if !job.advance(budget) {
        let next = (*serial).max(id + 1);
        *serial = next + 1;
        jobs.insert(next, job);
    }
}
struct Entry {
    epoch: u64,
    cut: Option<usize>,
    engine: MathEngine,
    focus: Option<MathFocus>,
}
struct State {
    document: DocumentId,
    revision: u64,
    engines: BTreeMap<(MathObjectId, bool), Entry>,
    retired: BTreeMap<usize, Retired>,
    serial: usize,
    maintenance_turn: bool,
}
#[derive(Clone)]
pub struct SharedMath(Rc<RefCell<State>>);
impl SharedMath {
    pub fn new(document: &Document) -> Self {
        Self(Rc::new(RefCell::new(State {
            document: document.id(),
            revision: document.revision(),
            engines: BTreeMap::new(),
            retired: BTreeMap::new(),
            serial: 0,
            maintenance_turn: false,
        })))
    }
    /// Native change notification: moves owners only; expensive destruction is
    /// paid by retire(). Prefix-preserving appends keep their parser and pages.
    pub fn apply(&self, document: &Document, update: &mdtui3::Update) -> Result<(), LayoutError> {
        let mut s = self.0.borrow_mut();
        if s.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if update.revision != document.revision() {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if s.revision == update.revision {
            return Ok(());
        }
        if s.revision != update.base_revision {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        s.revision = update.revision;
        for change in &update.math_changes {
            for prefix in [false, true] {
                let key = (change.id, prefix);
                if s.engines
                    .get(&key)
                    .is_some_and(|e| change.after.is_none_or(|m| m.epoch != e.epoch))
                {
                    let entry = s.engines.remove(&key).unwrap();
                    entry.engine.invalidate_payloads();
                    let serial = s.serial;
                    s.serial += 1;
                    s.retired.insert(
                        serial,
                        Retired::Engine(Box::new(entry.engine.into_retirement())),
                    );
                }
            }
        }

        Ok(())
    }
    pub(crate) fn maintenance(&self, budget: &mut Budget) {
        let mut s = self.0.borrow_mut();
        s.maintenance_turn = !s.maintenance_turn;
        if !s.maintenance_turn
            || s.retired.is_empty()
            || budget.remaining().queries == 0
            || budget.remaining().objects == 0
        {
            return;
        }
        let (id, mut job) = s.retired.pop_first().unwrap();
        if !job.advance(budget) {
            let serial = s.serial.max(id + 1);
            s.serial = serial + 1;
            s.retired.insert(serial, job);
        }
    }
    pub fn retire(&self, budget: &mut Budget) -> bool {
        let mut s = self.0.borrow_mut();
        let count = s.retired.len();
        // One round avoids blocking new objects on payloads still retained by
        // the previous screen/ACK/clipboard. No work runs outside Budget.
        for _ in 0..count {
            if budget.remaining().queries == 0 || budget.remaining().objects == 0 {
                return false;
            }
            let (id, mut job) = s.retired.pop_first().unwrap();
            if !job.advance(budget) {
                let serial = s.serial.max(id + 1);
                s.serial = serial + 1;
                s.retired.insert(serial, job);
            }
        }
        s.retired.is_empty()
    }
    /// Retire the single surface's active engines as well as obsolete queries.
    pub(crate) fn retire_all(&self, budget: &mut Budget) -> bool {
        {
            let mut s = self.0.borrow_mut();
            while !s.engines.is_empty() {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return false;
                }
                let (_, entry) = s.engines.pop_first().unwrap();
                entry.engine.invalidate_payloads();
                let serial = s.serial;
                s.serial += 1;
                s.retired.insert(
                    serial,
                    Retired::Engine(Box::new(entry.engine.into_retirement())),
                );
            }
        }
        self.retire(budget)
    }
    pub fn retained_owners(&self) -> usize {
        self.0.borrow().engines.len()
    }
    pub fn retirement_jobs(&self) -> usize {
        self.0.borrow().retired.len()
    }
    pub fn stats(&self, id: MathObjectId) -> Option<MathStats> {
        let state = self.0.borrow();
        state
            .engines
            .get(&(id, false))
            .or_else(|| state.engines.get(&(id, true)))
            .map(|e| e.engine.stats())
    }
    /// Preserve a glyph's complete source atom while choosing its intrinsic
    /// position (operators and fences can project several cells from one atom).
    /// All foreign/stale checks precede cache access and Budget consumption.
    pub fn set_focus(
        &self,
        document: &Document,
        focus: MathFocus,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        let mut state = self.0.borrow_mut();
        if state.document != document.id() || focus.key.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if document
            .math_object(focus.key.object)
            .is_none_or(|meta| meta.epoch != focus.key.epoch || meta.version != focus.key.version)
        {
            return Err(LayoutError::StaleContinuation);
        }
        let entry = state
            .engines
            .get_mut(&(focus.key.object, focus.prefix))
            .ok_or(LayoutError::StaleContinuation)?;
        if entry.engine.owner != focus.key.owner {
            return Err(LayoutError::StaleContinuation);
        }
        if entry.engine.snapshot_key() != focus.key || !entry.engine.ready {
            return Err(LayoutError::StaleContinuation);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return Ok(false);
        }
        entry.focus = Some(focus);
        Ok(true)
    }
    pub fn query(
        &self,
        document: &Document,
        meta: MathObjectMeta,
        rect: Rect,
    ) -> Result<MathQuery, LayoutError> {
        self.query_at_source_cut(document, meta, rect, None)
    }
    pub fn query_at_source_cut(
        &self,
        document: &Document,
        meta: MathObjectMeta,
        rect: Rect,
        cut: Option<usize>,
    ) -> Result<MathQuery, LayoutError> {
        if self.0.borrow().document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if document.math_object(meta.id) != Some(meta) {
            return Err(LayoutError::StaleContinuation);
        }
        if cut.is_some_and(|at| !document.source().is_char_boundary(at)) {
            return Err(LayoutError::InvalidRange);
        }
        let cut = cut
            .filter(|&at| at < meta.content.end)
            .map(|at| at.max(meta.content.start));
        Ok(MathQuery {
            cut,
            owner: None,
            retire_turn: false,
            source_work: 0,
            shared: self.clone(),
            meta,
            rect,
            paint: None,
            view: None,
            ready: None,
            geometry: None,
            goal: None,
            locator: None,
        })
    }
}
pub struct MathQuery {
    cut: Option<usize>,
    owner: Option<super::EngineOwner>,
    retire_turn: bool,
    source_work: usize,
    shared: SharedMath,
    meta: MathObjectMeta,
    rect: Rect,
    paint: Option<MathPaint>,
    view: Option<MathView>,
    ready: Option<Rc<MathView>>,
    geometry: Option<Metrics>,
    goal: Option<usize>,
    locator: Option<super::MathLocator>,
}
impl MathQuery {
    /// Project a root-source goal through the indexed AST before painting.
    /// The source remains authoritative when screen columns or width change.
    pub fn with_source(mut self, source: Option<usize>) -> Self {
        self.goal = source;
        self
    }
    pub fn source_work(&self) -> usize {
        self.source_work
    }
    pub fn output_storage(&self) -> super::StorageWork {
        self.view
            .as_ref()
            .map(|v| v.cells.storage())
            .or_else(|| self.ready.as_ref().map(|v| v.cells.storage()))
            .unwrap_or_default()
    }
    pub fn paint_work(&self) -> Option<super::PaintWork> {
        self.paint.as_ref().map(|p| p.work())
    }
    pub fn result(&self) -> Option<Rc<MathView>> {
        self.ready.clone()
    }
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        let mut s = self.shared.0.borrow_mut();
        if s.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if document.math_object(self.meta.id) != Some(self.meta) {
            return Err(LayoutError::StaleContinuation);
        }
        let key = (self.meta.id, self.cut.is_some());
        if [false, true].into_iter().any(|prefix| {
            s.engines
                .get(&(self.meta.id, prefix))
                .is_some_and(|e| e.epoch != self.meta.epoch)
        }) {
            return Err(LayoutError::StaleContinuation);
        }
        if let Some(owner) = self.owner
            && s.engines
                .get(&key)
                .is_none_or(|entry| entry.engine.owner != owner)
        {
            return Err(LayoutError::StaleContinuation);
        }
        if self.ready.is_some() {
            return Ok(true);
        }
        if s.engines.get(&key).is_some_and(|e| e.cut != self.cut) {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            let old = s.engines.remove(&key).unwrap();
            let live = old.engine.shaper.live.clone();
            let mut engine = match self.cut {
                Some(cut) => MathEngine::new_at_source_cut(document, self.meta, cut)?,
                None => MathEngine::new(document, self.meta)?,
            };
            engine.shaper.live = live;
            s.engines.insert(
                key,
                Entry {
                    epoch: self.meta.epoch,
                    cut: self.cut,
                    engine,
                    focus: None,
                },
            );
            let serial = s.serial;
            s.serial += 1;
            s.retired.insert(
                serial,
                Retired::Engine(Box::new(old.engine.into_retirement())),
            );
        }
        if !s.engines.contains_key(&key) {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            let live = s
                .engines
                .get(&(self.meta.id, !key.1))
                .map(|entry| entry.engine.shaper.live.clone());
            let mut engine = match self.cut {
                Some(cut) => MathEngine::new_at_source_cut(document, self.meta, cut)?,
                None => MathEngine::new(document, self.meta)?,
            };
            if let Some(live) = live {
                engine.shaper.live = live;
            }
            s.engines.insert(
                key,
                Entry {
                    epoch: self.meta.epoch,
                    cut: self.cut,
                    engine,
                    focus: None,
                },
            );
        }
        let State {
            engines,
            retired,
            serial,
            ..
        } = &mut *s;
        let entry = engines.get_mut(&key).unwrap();
        self.owner.get_or_insert(entry.engine.owner);
        if entry.epoch != self.meta.epoch {
            return Err(LayoutError::StaleContinuation);
        }
        while self.geometry.is_none() {
            if budget.remaining().queries > 0 && budget.remaining().objects > 0 {
                self.retire_turn = !self.retire_turn;
                if self.retire_turn {
                    retire_one(retired, serial, budget);
                }
            }
            let before = entry.engine.decoder.input.source_bytes;
            let status = entry.engine.advance_quantum(document, budget)?;
            self.source_work += entry.engine.decoder.input.source_bytes - before;
            match status {
                MathProgress::Ready | MathProgress::PendingInput => {
                    self.geometry = Some(entry.engine.metrics().ok_or(LayoutError::InvalidRange)?)
                }
                MathProgress::Preparing => continue,
                MathProgress::BudgetExhausted => return Ok(false),
                MathProgress::NeedShape => return Err(LayoutError::InvalidRange),
            }
        }
        if let Some(goal) = self.goal
            && let Some(focus) = entry.focus.filter(|f| {
                f.key == entry.engine.snapshot_key()
                    && f.source.start <= goal
                    && goal < f.source.end
            })
        {
            self.rect.y = focus.row;
            self.rect.x = focus
                .column
                .saturating_sub(self.rect.width.saturating_sub(1) / 2);
            self.goal = None;
        }
        if let Some(goal) = self.goal {
            if self.locator.is_none() {
                self.locator = Some(entry.engine.locate_source(Span::new(goal, goal))?);
            }
            loop {
                match self
                    .locator
                    .as_mut()
                    .unwrap()
                    .advance(&entry.engine, document, budget)?
                {
                    super::LocateStep::Position(position) => {
                        self.rect.y = position.row;
                        self.rect.x = position
                            .column
                            .saturating_sub(self.rect.width.saturating_sub(1) / 2);
                        self.goal = None;
                        self.locator = None;
                        break;
                    }
                    super::LocateStep::Progress => {}
                    super::LocateStep::BudgetExhausted => return Ok(false),
                }
            }
        }
        if self.paint.is_none() {
            let metrics = self.geometry.unwrap();
            self.paint = Some(entry.engine.paint(self.rect)?);
            self.view = Some(MathView {
                meta: self.meta,
                key: entry.engine.snapshot_key(),
                metrics,
                has_ink: entry.engine.has_ink(),
                rect: self.rect,
                source_cut: self.cut,
                cells: ViewCells::default(),
            });
        }
        loop {
            if budget.remaining().queries > 0 && budget.remaining().objects > 0 {
                self.retire_turn = !self.retire_turn;
                if self.retire_turn {
                    retire_one(retired, serial, budget);
                }
            }
            match self
                .paint
                .as_mut()
                .unwrap()
                .advance(&entry.engine, document, budget)?
            {
                PaintStep::Progress => {}
                PaintStep::Cell(cell) => self.view.as_mut().unwrap().cells.push(ViewCell {
                    x: cell.x,
                    y: cell.y,
                    width: cell.width,
                    source: cell.source,
                    text: entry.engine.text_payload(cell.ink)?,
                }),
                PaintStep::BudgetExhausted => return Ok(false),
                PaintStep::Complete => {
                    if !self.paint.as_mut().unwrap().retire_step(budget) {
                        return Ok(false);
                    }
                    self.paint = None;
                    self.ready = Some(Rc::new(self.view.take().unwrap()));
                    return Ok(true);
                }
            }
        }
    }
}

impl Drop for MathQuery {
    fn drop(&mut self) {
        if self.paint.is_none() && self.view.is_none() && self.ready.is_none() {
            return;
        }
        let mut s = self.shared.0.borrow_mut();
        let serial = s.serial;
        s.serial += 1;
        s.retired.insert(
            serial,
            Retired::Query(Box::new(RetiredQuery {
                paint: self.paint.take(),
                view: self.view.take(),
                ready: self.ready.take(),
            })),
        );
    }
}
