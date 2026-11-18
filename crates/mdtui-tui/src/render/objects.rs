//! One revision memo shared by display, object selection and layout. No task
//! owns historical trees: cancellation drops empty core cursors, and the next
//! budgeted operation retires the sole old memo before rebinding it.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mdtui3::{
    Budget, Completion, Document, LayoutError, LinkActivity, LinkDisposition, LinkId, LinkKind,
    LinkResolutionWork, LinkResolver, LinkStep, LinkWindow, Span, WorkLimit,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Presentation {
    pub disposition: Option<LinkDisposition>,
    pub image: Option<LinkId>,
    pub hit: Option<LinkId>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObjectStats {
    pub memo_entries: usize,
    pub presentation_entries: usize,
    pub presentation_steps: u64,
    pub retired_presentations: u64,
    pub resolution: LinkResolutionWork,
    pub styles: mdtui3::StyleResolutionWork,
    pub style_memo_entries: usize,
}

#[derive(Clone)]
pub struct SharedLinks(Rc<RefCell<LinkState>>);

pub struct LinkState {
    math: super::math::SharedMath,
    shape: Option<super::shape::SharedShape>,
    seed: Option<super::shape::ShapeSeed>,
    pub resolver: LinkResolver,
    document: mdtui3::DocumentId,
    revision: u64,
    retirement_requested: bool,
    active_tasks: usize,
    fragment_titles: bool,
    presentations: BTreeMap<LinkId, Presentation>,
    literals: BTreeMap<usize, LiteralSyntax>,
    steps: u64,
    retired: u64,
}

fn charge(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    })
}

impl SharedLinks {
    pub fn new(document: &Document) -> Self {
        Self(Rc::new(RefCell::new(LinkState {
            math: super::math::SharedMath::new(document),
            resolver: document.link_resolver(),
            shape: None,
            seed: None,
            document: document.id(),
            revision: document.revision(),
            retirement_requested: false,
            active_tasks: 0,
            fragment_titles: false,
            presentations: BTreeMap::new(),
            literals: BTreeMap::new(),
            steps: 0,
            retired: 0,
        })))
    }

    pub fn math(&self) -> super::math::SharedMath {
        self.0.borrow().math.clone()
    }

    pub fn set_shape(&self, shape: super::shape::SharedShape) -> Result<(), LayoutError> {
        let mut state = self.0.borrow_mut();
        if shape.document_id() != state.document || state.resolver.document_id() != state.document {
            return Err(LayoutError::WrongDocument);
        }
        if let Some(current) = &state.shape {
            return if current.same_index(&shape) {
                Ok(())
            } else {
                Err(LayoutError::WrongDocument)
            };
        }
        if shape.applied_revision() != state.revision {
            return Err(LayoutError::StaleContinuation);
        }
        state.shape = Some(shape);
        Ok(())
    }
    pub(crate) fn set_seed(&self, seed: super::shape::ShapeSeed) {
        self.0.borrow_mut().seed = Some(seed);
    }
    pub(crate) fn retire_initialization(&self, budget: &mut Budget) -> bool {
        let mut state = self.0.borrow_mut();
        let shape = state.shape.clone();
        if let Some(seed) = state.seed.as_mut() {
            if seed.retire(
                shape.as_ref().expect("seed retains its shape owner"),
                budget,
            ) != Completion::Covered
            {
                return false;
            }
            state.seed = None;
        }
        true
    }
    pub fn shape_ready(&self) -> bool {
        let state = self.0.borrow();
        state.shape.is_some() && state.seed.is_none()
    }
    pub fn prepare_shape(
        &self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        self.prepare_shape_inner(document, budget, false)
    }
    pub(crate) fn prepare_shape_indexed(
        &self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        self.prepare_shape_inner(document, budget, true)
    }
    fn prepare_shape_inner(
        &self,
        document: &Document,
        budget: &mut Budget,
        indexed: bool,
    ) -> Result<bool, LayoutError> {
        let mut state = self.0.borrow_mut();
        if state.document != document.id() || state.resolver.document_id() != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if state
            .shape
            .as_ref()
            .is_some_and(|shape| shape.document_id() != document.id())
        {
            return Err(LayoutError::WrongDocument);
        }
        if state.shape.is_none() {
            if indexed && (budget.remaining().queries == 0 || budget.remaining().objects == 0) {
                return Ok(false);
            }
            state.shape = Some(super::shape::SharedShape::from_current(document));
            state.seed = Some(if indexed {
                super::shape::ShapeSeed::indexed(document)
            } else {
                super::shape::ShapeSeed::new(document)
            });
        }
        let shape = state.shape.as_ref().unwrap().clone();
        if let Some(seed) = state.seed.as_mut() {
            if !seed.advance(&shape, document, budget)? {
                return Ok(false);
            }
            state.seed = None;
        }
        Ok(true)
    }
    pub(crate) fn apply_shape_update(
        &self,
        document: &Document,
        update: &mdtui3::Update,
    ) -> Result<(), LayoutError> {
        self.math().apply(document, update)?;
        let mut state = self.0.borrow_mut();
        if state.document != document.id() || state.resolver.document_id() != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if state.revision == update.base_revision
            && state.revision != update.revision
            && state.resolver.rebind_current_slugs(document, update)?
        {
            state.revision = update.revision;
        }
        let Some(shape) = state.shape.as_ref() else {
            return Ok(());
        };
        if state.seed.is_some() {
            if shape.source_len() == 0 {
                state.shape = None;
                state.seed = None;
                return Ok(());
            }
            return Err(LayoutError::StaleContinuation);
        }
        if shape.applied_revision() != update.revision {
            shape.apply(document, update)?;
        }
        Ok(())
    }

    pub(crate) fn can_rebind(&self, document: &Document, revision: u64) -> bool {
        self.0.borrow().resolver.can_rebind(document, revision)
    }

    pub fn shape(&self) -> Option<super::shape::SharedShape> {
        self.0.borrow().shape.clone()
    }

    pub fn prepare(&self, document: &Document, budget: &mut Budget) -> Result<bool, LayoutError> {
        let mut state = self.0.borrow_mut();
        if state.document != document.id() || state.resolver.document_id() != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if state.document == document.id()
            && state.revision == document.revision()
            && (!state.retirement_requested || state.active_tasks != 0)
        {
            return Ok(true);
        }
        if state.resolver.retire(budget) != Completion::Covered {
            return Ok(false);
        }
        while !state.presentations.is_empty() || !state.literals.is_empty() {
            if !charge(budget) {
                return Ok(false);
            }
            if state.presentations.pop_first().is_none() {
                state.literals.pop_first();
            }
            state.retired += 1;
        }
        state.resolver.reset(document)?;
        state.revision = document.revision();
        state.retirement_requested = false;
        Ok(true)
    }

    pub(crate) fn enable_fragment_titles(&self) -> bool {
        let mut state = self.0.borrow_mut();
        let changed = !state.fragment_titles;
        state.fragment_titles = true;
        changed
    }
    pub(crate) fn fragment_titles(&self) -> bool {
        self.0.borrow().fragment_titles
    }
    pub(crate) fn retire_current(&self, b: &mut Budget) -> Completion {
        let mut state = self.0.borrow_mut();
        if state.resolver.retire(b) != Completion::Covered {
            return Completion::BudgetExhausted;
        }
        while !state.presentations.is_empty() || !state.literals.is_empty() {
            if !charge(b) {
                return Completion::BudgetExhausted;
            }
            if state.presentations.pop_first().is_none() {
                state.literals.pop_first();
            }
            state.retired += 1;
        }
        Completion::Covered
    }
    pub fn hold(&self) -> LinkLease {
        self.0.borrow_mut().active_tasks += 1;
        LinkLease(self.clone())
    }

    pub fn request_retirement(&self) {
        self.0.borrow_mut().retirement_requested = true;
    }

    pub fn retirement_pending(&self) -> bool {
        let state = self.0.borrow();
        state.retirement_requested && state.active_tasks == 0
    }

    pub fn with_resolver<T>(&self, apply: impl FnOnce(&mut LinkResolver) -> T) -> T {
        apply(&mut self.0.borrow_mut().resolver)
    }
    pub(crate) fn check_shape_context(
        &self,
        document: &Document,
        shape: &super::shape::SharedShape,
    ) -> Result<(), LayoutError> {
        let state = self.0.borrow();
        if state.document != document.id()
            || state.resolver.document_id() != document.id()
            || state
                .shape
                .as_ref()
                .is_some_and(|current| !current.same_index(shape))
        {
            return Err(LayoutError::WrongDocument);
        }
        Ok(())
    }

    pub fn presentation(&self, id: LinkId) -> Presentation {
        self.0
            .borrow()
            .presentations
            .get(&id)
            .copied()
            .unwrap_or_default()
    }

    pub fn stats(&self) -> ObjectStats {
        let state = self.0.borrow();
        ObjectStats {
            memo_entries: state.resolver.memo_entries(),
            presentation_entries: state.presentations.len() + state.literals.len(),
            presentation_steps: state.steps,
            retired_presentations: state.retired,
            resolution: state.resolver.work(),
            styles: state.resolver.style_work(),
            style_memo_entries: state.resolver.style_memo_entries(),
        }
    }
}

/// A pending task keeps same-revision work alive until it can finish. Revision
/// retirement takes precedence; cancelling a task only releases this counter.
pub struct LinkLease(SharedLinks);
impl Drop for LinkLease {
    fn drop(&mut self) {
        self.0.0.borrow_mut().active_tasks -= 1;
    }
}

struct Pending {
    id: LinkId,
    current: Option<LinkId>,
    path: Vec<(LinkId, LinkDisposition)>,
    activity: Option<LinkActivity>,
    disposition: Option<LinkDisposition>,
    base: Presentation,
    unwinding: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ImagePlaceholder {
    pub source: Span,
    pub hit: LinkId,
}

#[derive(Debug, Clone, Copy)]
pub struct LiteralSyntax {
    pub source: Span,
    pub hit: Option<LinkId>,
    pub image: bool,
}

pub struct ObjectWindow {
    pub(crate) outer_images: Vec<(LinkId, Span, Span)>,
    source: Span,
    literal_image_ranges: Vec<Span>,
    composite_images: Vec<(Span, ImagePlaceholder)>,
    fallback_cursor: usize,
    pub empty_fallbacks: Vec<ImagePlaceholder>,
    window: LinkWindow,
    pending: Option<Pending>,
    links: SharedLinks,
    pub placeholders: Vec<ImagePlaceholder>,
    complete: bool,
    revision: u64,
    preserved: Option<u64>,
    restart: bool,
}

impl ObjectWindow {
    pub fn new(document: &Document, source: Span, links: SharedLinks) -> Option<Self> {
        Some(Self {
            outer_images: Vec::new(),
            source,
            literal_image_ranges: Vec::new(),
            composite_images: Vec::new(),
            fallback_cursor: 0,
            empty_fallbacks: Vec::new(),
            window: document.link_window(source),
            pending: None,
            links,
            placeholders: Vec::new(),
            complete: false,
            revision: document.revision(),
            preserved: None,
            restart: false,
        })
    }

    /// The caller proved this source and its enclosing object semantics stable.
    /// Current-slug updates can invalidate native continuations even when this
    /// table cell is unchanged. Retire that paused guard before restarting it.
    pub(crate) fn rebind_stable(&mut self, document: &Document) {
        if self.complete {
            self.preserved = Some(document.revision());
        } else if !self.links.can_rebind(document, self.revision) {
            self.restart = true;
        }
        self.revision = document.revision();
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if !self.links.prepare(document, budget)? {
            return Ok(false);
        }
        if self.complete && self.preserved == Some(document.revision()) {
            return Ok(true);
        }
        if self.restart {
            if let Some(activity) = self.pending.as_mut().and_then(|p| p.activity.as_mut())
                && activity.retire_continuation(budget) != Completion::Covered
            {
                return Ok(false);
            }
            if !charge(budget) {
                return Ok(false);
            }
            self.pending = None;
            self.outer_images.clear();
            self.literal_image_ranges.clear();
            self.composite_images.clear();
            self.empty_fallbacks.clear();
            self.placeholders.clear();
            self.fallback_cursor = 0;
            self.window = document.link_window(self.source);
            self.restart = false;
        }
        self.links
            .with_resolver(|r| self.window.rebind_with(document, r))?;
        while !self.complete {
            if self.pending.is_none() {
                match self.window.advance(document, budget)? {
                    LinkStep::Object(id, _) => {
                        self.pending = Some(Pending {
                            id,
                            current: Some(id),
                            path: Vec::new(),
                            activity: None,
                            disposition: None,
                            base: Presentation::default(),
                            unwinding: false,
                        })
                    }
                    LinkStep::BudgetExhausted => return Ok(false),
                    LinkStep::Complete => {
                        self.complete = true;
                        break;
                    }
                }
            }
            let pending = self.pending.as_mut().expect("pending object");
            loop {
                if pending.unwinding {
                    let Some(&(id, disposition)) = pending.path.last() else {
                        break;
                    };
                    if !charge(budget) {
                        return Ok(false);
                    }
                    let mut state = self.links.0.borrow_mut();
                    if pending.base.image.is_none() && disposition == LinkDisposition::Active {
                        let info = document.link_info(id).ok_or(LayoutError::InvalidRange)?;
                        if info.kind == LinkKind::Image {
                            pending.base.image = Some(id);
                            pending.base.hit = pending.base.hit.or(Some(id));
                        } else {
                            pending.base.hit = Some(id);
                        }
                    }
                    pending.base.disposition = Some(disposition);
                    state.presentations.insert(id, pending.base);
                    state.steps += 1;
                    pending.path.pop();
                    continue;
                }
                let Some(id) = pending.current else {
                    pending.unwinding = true;
                    continue;
                };
                if pending.activity.is_none() {
                    if !charge(budget) {
                        return Ok(false);
                    }
                    let state = self.links.0.borrow();
                    if let Some(&presentation) = state.presentations.get(&id) {
                        pending.base = presentation;
                        pending.unwinding = true;
                        continue;
                    }
                    pending.activity = document.link_activity(id);
                }
                if pending.disposition.is_none() {
                    pending.disposition = self.links.with_resolver(|resolver| {
                        pending
                            .activity
                            .as_mut()
                            .expect("valid object")
                            .rebind_with(document, resolver)?;
                        pending
                            .activity
                            .as_mut()
                            .expect("valid object")
                            .advance_with(document, resolver, budget)
                    })?;
                    if pending.disposition.is_none() {
                        return Ok(false);
                    }
                }
                if !charge(budget) {
                    return Ok(false);
                }
                pending.path.push((id, pending.disposition.take().unwrap()));
                pending.current = document.link_parent(id);
                pending.activity = None;
            }
            let id = pending.id;
            let presentation = self.links.presentation(id);
            let info = document.link_info(id).ok_or(LayoutError::InvalidRange)?;
            if presentation.image == Some(id) {
                self.outer_images.push((id, info.source, info.label));
            }
            if presentation.disposition == Some(LinkDisposition::Unresolved) {
                if info.kind == LinkKind::Image {
                    if let Some(last) = self
                        .literal_image_ranges
                        .last_mut()
                        .filter(|last| info.source.start <= last.end)
                    {
                        last.end = last.end.max(info.source.end);
                    } else {
                        self.literal_image_ranges.push(info.source);
                    }
                }
                let mut state = self.links.0.borrow_mut();
                for source in [
                    Span::new(info.source.start, info.label.start),
                    Span::new(info.label.end, info.source.end),
                ] {
                    if !source.is_empty() {
                        state.literals.insert(
                            source.start,
                            LiteralSyntax {
                                source,
                                hit: presentation.hit,
                                image: presentation.image.is_some(),
                            },
                        );
                    }
                }
            }
            if presentation.image == Some(id)
                && !info.label.is_empty()
                && info.label.start >= self.source.start
                && info.label.end <= self.source.end
                && info.source.start >= self.source.start
            {
                self.composite_images.push((
                    info.label,
                    ImagePlaceholder {
                        source: Span::new(info.source.start, info.source.start + 1),
                        hit: presentation.hit.expect("active image"),
                    },
                ));
            }
            if presentation.image == Some(id) && info.label.is_empty() {
                self.placeholders.push(ImagePlaceholder {
                    source: Span::new(info.source.start, info.source.start + 1),
                    hit: presentation.hit.expect("active image"),
                });
            }
            self.pending = None;
        }
        Ok(true)
    }

    /// Only complete labels inside this prepared window can establish an empty
    /// displayed alt. No extra label scan and no nested empty-image control.
    pub fn finish_empty(&mut self, budget: &mut Budget, has_text: impl Fn(Span) -> bool) -> bool {
        while let Some(&(label, placeholder)) = self.composite_images.get(self.fallback_cursor) {
            if !charge(budget) {
                return false;
            }
            if !has_text(label) {
                self.empty_fallbacks.push(placeholder);
            }
            self.fallback_cursor += 1;
        }
        true
    }

    pub(crate) fn scratch_capacity_bytes(&self) -> usize {
        use std::mem::size_of;
        self.literal_image_ranges.capacity() * size_of::<Span>()
            + self.outer_images.capacity() * size_of::<(LinkId, Span, Span)>()
            + self.composite_images.capacity() * size_of::<(Span, ImagePlaceholder)>()
            + (self.placeholders.capacity() + self.empty_fallbacks.capacity())
                * size_of::<ImagePlaceholder>()
            + self.pending.as_ref().map_or(0, |pending| {
                pending.path.capacity() * size_of::<(LinkId, LinkDisposition)>()
            })
    }

    pub fn is_literal_image(&self, source: Span) -> bool {
        let at = self
            .literal_image_ranges
            .partition_point(|span| span.end <= source.start);
        self.literal_image_ranges
            .get(at)
            .is_some_and(|span| span.start < source.end)
    }

    pub fn index_work(&self) -> mdtui3::LinkWindowWork {
        self.window.work()
    }

    pub fn literal_syntax(&self, source: Span) -> Vec<LiteralSyntax> {
        let state = self.links.0.borrow();
        let before = state
            .literals
            .range(..source.start)
            .next_back()
            .map(|(_, syntax)| *syntax);
        before
            .into_iter()
            .chain(
                state
                    .literals
                    .range(source.start..source.end)
                    .map(|(_, syntax)| *syntax),
            )
            .filter_map(|syntax| {
                let start = syntax.source.start.max(source.start);
                let end = syntax.source.end.min(source.end);
                (start < end).then_some(LiteralSyntax {
                    source: Span::new(start, end),
                    hit: syntax.hit,
                    image: syntax.image,
                })
            })
            .collect()
    }

    pub fn presentation(&self, id: LinkId) -> Presentation {
        self.links.presentation(id)
    }
}
