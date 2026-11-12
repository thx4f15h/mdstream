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
    pub resolver: LinkResolver,
    document: mdtui3::DocumentId,
    revision: u64,
    retirement_requested: bool,
    active_tasks: usize,
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
            resolver: document.link_resolver(),
            document: document.id(),
            revision: document.revision(),
            retirement_requested: false,
            active_tasks: 0,
            presentations: BTreeMap::new(),
            literals: BTreeMap::new(),
            steps: 0,
            retired: 0,
        })))
    }

    pub fn prepare(&self, document: &Document, budget: &mut Budget) -> Result<bool, LayoutError> {
        let mut state = self.0.borrow_mut();
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
        state.document = document.id();
        state.revision = document.revision();
        state.retirement_requested = false;
        Ok(true)
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
        })
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if !self.links.prepare(document, budget)? {
            return Ok(false);
        }
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
