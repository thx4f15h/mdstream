//! View-local line policies feed only the existing paragraph SourceFold IDs.
use super::{
    checkpoint::RenderCheckpoint,
    objects::SharedLinks,
    prefix::{DisplayPrefix, LogicalPrefix, PrefixLines},
};
use mdtui3::{
    BlockId, Budget, Completion, Document, FoldId, FoldView, LayoutError, ParagraphFoldCandidate,
    ParagraphFoldCandidates, SourceFoldId, SourceFoldRegistration, Span, Update, WorkLimit,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound::{Excluded, Unbounded},
};
type View = FoldView<RenderCheckpoint>;
enum Query {
    Logical(Box<LogicalPrefix>),
    Display(Box<DisplayPrefix>),
}
struct Policy {
    mode: PrefixLines,
    id: Option<SourceFoldId>,
    candidate: ParagraphFoldCandidate,
    query: Option<Query>,
    registration: Option<SourceFoldRegistration>,
    cut: Option<usize>,
    proof: Option<(Span, (usize, u64, u64))>,
    width: u16,
    collapse: bool,
    starts: usize,
    delivered: bool,
}
#[derive(Default)]
pub(crate) struct ParagraphPrefixes {
    entries: BTreeMap<BlockId, Policy>,
    dirty: BTreeSet<BlockId>,
    all: bool,
    mandatory: bool,
    cancel_unpublished: bool,
    after: Option<BlockId>,
    active: Option<BlockId>,
    width: u16,
    query_starts: usize,
    pub default: Option<PrefixLines>,
    discovery: Option<(ParagraphFoldCandidates, PrefixLines, bool, Span)>,
    range: Option<Span>,
    pub selected: Option<FoldId>,
    pub message: String,
}
fn paid(b: &mut Budget) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
impl ParagraphPrefixes {
    pub fn query_starts(&self) -> usize {
        self.query_starts
    }
    pub fn pending(&self) -> bool {
        self.mandatory
            || self.discovery.is_some()
            || self.range.is_some()
            || self.active.is_some()
            || self.all
            || !self.dirty.is_empty()
    }
    pub fn validation_pending(&self) -> bool {
        self.mandatory
    }
    pub fn cancel(&mut self) {
        self.discovery = None;
        self.range = None;
        self.cancel_unpublished = true;
        if let Some(block) = self.active {
            let unpublished = self
                .entries
                .get(&block)
                .is_some_and(|p| !p.delivered && p.id.is_none());
            if unpublished {
                self.entries.remove(&block);
                self.active = None;
            } else if !self.mandatory {
                if let Some(policy) = self.entries.get_mut(&block) {
                    policy.collapse = false;
                    policy.delivered = true;
                }
                self.active = None;
            }
        }
        if !self.mandatory {
            self.all = false;
            self.after = None;
            self.dirty.clear();
        }
    }
    pub fn request(
        &mut self,
        doc: &Document,
        source: usize,
        mode: PrefixLines,
    ) -> Result<(), LayoutError> {
        let end = source.saturating_add(1).min(doc.source().len());
        let range = Span::new(source.min(end), end);
        self.discovery = Some((
            ParagraphFoldCandidates::new(doc, range, 0)?,
            mode,
            false,
            range,
        ));
        self.message = "body-prefix discovery pending".into();
        Ok(())
    }
    pub fn on_update(
        &mut self,
        doc: &Document,
        view: &View,
        update: &Update,
    ) -> Result<(), LayoutError> {
        if let Some(block) = self.active.take() {
            self.dirty.insert(block);
        }
        if let Some((_, mode, automatic, old)) = self.discovery.take() {
            if automatic {
                self.range = Some(self.range.map_or(old, |new| {
                    Span::new(old.start.min(new.start), old.end.max(new.end))
                }));
            } else {
                self.discovery = Some((
                    ParagraphFoldCandidates::new(doc, old, 0)?,
                    mode,
                    automatic,
                    old,
                ));
            }
        }
        // Only the sole growing paragraph needs ordinary-append maintenance.
        // Nonlocal binding changes and resize explicitly traverse policies by budget.
        if !self.entries.is_empty()
            && (!update.references_changed.is_empty() || !update.loosened_lists.is_empty())
        {
            self.all = true;
            self.after = None;
        }
        if let Some((&block, _)) = self
            .entries
            .iter()
            .next_back()
            .filter(|(_, p)| p.candidate.open)
        {
            self.dirty.insert(block);
        }
        self.mandatory |= self.all || !self.dirty.is_empty();
        if self.default.is_some() {
            let new = mdtui3::ParagraphAutoFold::for_update(doc, view, update)?.remaining_range();
            self.range = Some(self.range.map_or(new, |old| {
                Span::new(old.start.min(new.start), old.end.max(new.end))
            }));
        }
        Ok(())
    }
    pub fn resize(&mut self) {
        if let Some(block) = self.active.take() {
            self.dirty.insert(block);
        }
        if !self.entries.is_empty() {
            self.mandatory = true;
            self.all = true;
            self.after = None;
        }
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        view: &mut View,
        links: &SharedLinks,
        width: u16,
        budget: &mut Budget,
    ) -> Result<(), String> {
        if width != self.width {
            self.resize();
            self.width = width;
        }
        if let Some(block) = self.active {
            let Some(policy) = self.entries.get_mut(&block) else {
                self.active = None;
                return Ok(());
            };
            let before = policy.starts;
            let result = advance_policy(doc, view, links, width, budget, policy);
            self.query_starts += policy.starts - before;
            let complete = result.map_err(|e| e.to_string())?;
            self.message = format!(
                "body-prefix {:?}: {}..{} {}",
                policy.mode,
                policy.candidate.source.start,
                policy.candidate.source.end,
                if complete { "ready" } else { "pending" }
            );
            if complete {
                policy.delivered = true;
                self.selected = policy.id.map(FoldId::Source);
                self.active = None;
            }
            return Ok(());
        }
        let from_dirty = self.dirty.first().copied();
        let next = from_dirty.or_else(|| {
            self.all
                .then(|| {
                    self.entries
                        .range((self.after.map_or(Unbounded, Excluded), Unbounded))
                        .next()
                        .map(|(&b, _)| b)
                })
                .flatten()
        });
        if let Some(block) = next {
            if !paid(budget) {
                return Ok(());
            }
            self.dirty.remove(&block);
            if from_dirty.is_none() {
                self.after = Some(block);
            }
            if self.cancel_unpublished
                && self
                    .entries
                    .get(&block)
                    .is_some_and(|p| !p.delivered && p.id.is_none())
            {
                self.entries.remove(&block);
                return Ok(());
            }
            let Some(candidate) = doc.paragraph_fold_snapshot(block) else {
                self.entries.remove(&block);
                return Ok(());
            };
            let policy = self.entries.get_mut(&block).unwrap();
            if let Some(id) = policy.id
                && view
                    .paragraph_fold_block(doc, id)
                    .map_err(|e| e.to_string())?
                    != Some(block)
            {
                self.entries.remove(&block);
                return Ok(());
            }
            let proof_current = policy.proof.is_some_and(|(range, old)| {
                links.shape().is_some_and(|shape| {
                    let current = shape.payload_snapshot(range);
                    old.0 == current.0 && old.1 == current.1
                })
            });
            let resumable = match &policy.query {
                Some(Query::Logical(_)) => true,
                Some(Query::Display(q)) => policy.width == width && q.can_extend(),
                None => false,
            };
            let append_streaming = proof_current
                && resumable
                && candidate.classified_end >= policy.candidate.classified_end
                && policy.cut == Some(policy.candidate.classified_end);
            let stable = policy.width == width
                && policy
                    .cut
                    .is_some_and(|cut| cut < policy.candidate.classified_end)
                && policy.proof.is_some_and(|(range, old)| {
                    links.shape().is_some_and(|shape| {
                        let current = shape.payload_snapshot(range);
                        old.0 == current.0 && old.1 == current.1
                    })
                });
            policy.candidate = candidate;
            if append_streaming {
                let source = Span::new(candidate.source.start, candidate.classified_end);
                match policy.query.as_mut().unwrap() {
                    Query::Logical(q) => q.extend_append(doc, source),
                    Query::Display(q) => q.extend_append(doc, source),
                }
                .map_err(|e| e.to_string())?;
                policy.cut = None;
                policy.proof = None;
                self.active = Some(block);
            } else if !stable {
                policy.query = None;
                policy.registration = None;
                policy.cut = None;
                policy.proof = None;
                policy.width = width;
                self.active = Some(block);
            }
            return Ok(());
        }
        self.all = false;
        self.after = None;
        self.mandatory = false;
        self.cancel_unpublished = false;
        if self.discovery.is_none()
            && let Some(range) = self.range.take()
            && let Some(mode) = self.default
        {
            self.discovery = Some((
                ParagraphFoldCandidates::new(doc, range, 4096).map_err(|e| e.to_string())?,
                mode,
                true,
                range,
            ));
        }
        if let Some((discovery, mode, automatic, _)) = &mut self.discovery {
            let mut limit = budget.remaining();
            limit.queries = limit.queries.min(1);
            limit.objects = limit.objects.min(1);
            let mut work = Budget::new(limit);
            let mut out = Vec::new();
            let status = discovery
                .advance(doc, &mut work, &mut out)
                .map_err(|e| e.to_string())?;
            assert!(budget.charge(work.used()));
            if let Some(candidate) = out.first().copied() {
                let allowed = !*automatic
                    || (!self.entries.contains_key(&candidate.block)
                        && !view
                            .paragraph_auto_excluded(doc, candidate.block)
                            .map_err(|e| e.to_string())?);
                if allowed {
                    let mut id = None;
                    if let Some(existing) = view
                        .source_fold_at(doc, candidate.source.start)
                        .map_err(|e| e.to_string())?
                        && let FoldId::Source(source) = existing.id
                        && view
                            .paragraph_fold_block(doc, source)
                            .map_err(|e| e.to_string())?
                            == Some(candidate.block)
                    {
                        id = Some(source);
                    }
                    self.entries.insert(
                        candidate.block,
                        Policy {
                            mode: *mode,
                            id,
                            candidate,
                            query: None,
                            registration: None,
                            cut: None,
                            proof: None,
                            width,
                            collapse: id.is_none(),
                            starts: 0,
                            delivered: false,
                        },
                    );
                    self.active = Some(candidate.block);
                }
            }
            if status != Completion::BudgetExhausted {
                if out.is_empty() && self.active.is_none() {
                    self.message = "no classified paragraph at source; input may be pending".into();
                }
                self.discovery = None;
            }
            return Ok(());
        }
        Ok(())
    }
}
fn advance_policy(
    doc: &Document,
    view: &mut View,
    links: &SharedLinks,
    width: u16,
    budget: &mut Budget,
    policy: &mut Policy,
) -> Result<bool, super::first_frame::FirstFrameError> {
    if policy.cut.is_none() {
        let source = Span::new(
            policy.candidate.source.start,
            policy.candidate.classified_end,
        );
        if policy.query.is_none() {
            if !paid(budget) {
                return Ok(false);
            }
            policy.starts += 1;
            policy.query = Some(match policy.mode {
                PrefixLines::Logical(n) => {
                    Query::Logical(Box::new(LogicalPrefix::new(doc, source, n)?))
                }
                PrefixLines::Display(n) => {
                    Query::Display(Box::new(DisplayPrefix::new(doc, source, n, width)?))
                }
            });
        }
        policy.cut = match policy.query.as_mut().unwrap() {
            Query::Logical(q) => q.advance(doc, links, budget)?,
            Query::Display(q) => q.advance(doc, links, budget)?,
        };
        if policy.cut.is_none() {
            return Ok(false);
        }
        let end = match policy.query.as_ref().unwrap() {
            Query::Logical(q) => q.dependency_end(),
            Query::Display(q) => q.dependency_end(),
        };
        let range = Span::new(source.start, end);
        if let Some(shape) = links.shape() {
            policy.proof = Some((range, shape.payload_snapshot(range)));
        }
        let resumable = match &policy.query {
            Some(Query::Logical(_)) => true,
            Some(Query::Display(q)) => q.can_extend(),
            None => false,
        };
        if !resumable || policy.cut != Some(source.end) {
            policy.query = None;
        }
    }
    let cut = policy.cut.unwrap();
    if let Some(id) = policy.id {
        if view.set_paragraph_fold_cut(doc, id, cut, budget)?.is_none() {
            return Ok(false);
        }
    } else {
        if cut >= policy.candidate.classified_end {
            return Ok(true);
        }
        if policy.registration.is_none() {
            policy.registration = Some(SourceFoldRegistration::paragraph_overlay(
                doc,
                view,
                policy.candidate.block,
                cut - policy.candidate.source.start,
            )?);
        }
        if let Some(FoldId::Source(id)) = policy
            .registration
            .as_mut()
            .unwrap()
            .advance(doc, view, budget)?
            .id
        {
            policy.id = Some(id);
            policy.registration = None;
        } else {
            return Ok(false);
        }
    }
    if policy.collapse {
        if !paid(budget) {
            return Ok(false);
        }
        view.set_collapsed(doc, FoldId::Source(policy.id.unwrap()), true)?;
        policy.collapse = false;
    }
    Ok(true)
}
