//! Classified delimiter events and source-derived ordered pair identities.
//! Source ingestion is linear in events. Conditional scopes are interpreted
//! once in a shared revision resolver, without reading Markdown bytes.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Excluded, Unbounded};

use crate::link_semantics::{LinkActivity, LinkDisposition, Resolution};
use crate::{
    Budget, Completion, Document, DocumentId, LayoutError, LinkId, Span, Style, Work, WorkLimit,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InlineStyleKind {
    Emphasis,
    Strong,
    Strike,
    Code,
}
impl InlineStyleKind {
    pub fn style(self) -> Style {
        match self {
            Self::Emphasis => Style::EMPHASIS,
            Self::Strong => Style::STRONG,
            Self::Strike => Style::STRIKE,
            Self::Code => Style::CODE,
        }
    }
}

/// Document-local syntax identity, derived from consumed original markers.
/// Query order, viewport placement and memo allocation cannot change this ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StylePairId {
    pub open: Span,
    pub close: Span,
    pub kind: InlineStyleKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DelimiterRun {
    pub source: Span,
    pub kind: usize,
    pub can_open: bool,
    pub can_close: bool,
}
impl DelimiterRun {
    fn eligible(self, bucket: usize) -> bool {
        if self.kind >= 12 {
            return bucket == self.kind;
        }
        let base = self.kind / 6 * 6;
        let residue = self.kind % 3;
        bucket >= base
            && bucket < base + 6
            && !((self.can_open || bucket - base >= 3)
                && residue != 0
                && (bucket % 3 + residue).is_multiple_of(3))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boundary {
    Transparent,
    Fixed(bool),
    Link(LinkId),
}
impl Boundary {
    fn conditional(self) -> bool {
        matches!(self, Self::Link(_))
    }
}
#[derive(Clone, Copy, Debug)]
enum Event {
    Enter {
        boundary: Boundary,
    },
    Leave,
    Pair(StylePairId),
    Run {
        run: DelimiterRun,
        unless: Option<LinkId>,
    },
}
type EventKey = (usize, u8, usize);
#[derive(Default)]
struct Scope {
    source: Span,
    events: crate::rewind::Map<EventKey, Event>,
    labels: crate::rewind::Map<usize, (EventKey, Option<LinkId>)>,
    conditional: usize,
    runs: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    style: Style,
    emphasis: usize,
    strong: usize,
}
impl Counts {
    fn with(self, kind: InlineStyleKind) -> Self {
        Self {
            style: self.style.union(kind.style()),
            emphasis: self.emphasis + usize::from(kind == InlineStyleKind::Emphasis),
            strong: self.strong + usize::from(kind == InlineStyleKind::Strong),
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Pair {
    parent: Option<StylePairId>,
    counts: Counts,
}
#[derive(Default)]
struct Forest {
    pairs: crate::rewind::Map<StylePairId, Pair>,
    roots: crate::rewind::Map<usize, StylePairId>,
    boundaries: crate::rewind::Map<usize, Option<StylePairId>>,
    markers: crate::rewind::Map<usize, usize>,
}
#[derive(Clone, Copy)]
struct Insert {
    pair: StylePairId,
    gap: usize,
    started: bool,
}
impl Insert {
    fn new(pair: StylePairId) -> Self {
        Self {
            pair,
            gap: pair.open.end,
            started: false,
        }
    }
}
impl Forest {
    fn gap(&mut self, start: usize, end: usize, parent: StylePairId) {
        if start < end {
            if !self.boundaries.contains_key(&end) {
                self.boundaries.insert(end, None);
            }
            self.boundaries.insert(start, Some(parent));
        }
    }
    // One call touches a fixed number of BTree entries. Each immediate child
    // attaches once, so all inserts together take O(P log(P+1)), O(P) space.
    fn insert_step(&mut self, task: &mut Insert) -> bool {
        let p = task.pair;
        if !task.started {
            if self.pairs.contains_key(&p) {
                return true;
            }
            self.pairs.insert(
                p,
                Pair {
                    parent: None,
                    counts: Counts::default(),
                },
            );
            task.started = true;
            return false;
        }
        if let Some((&key, &child)) = self.roots.range(p.open.start..).next()
            && key < p.close.end
        {
            debug_assert!(
                p.open.end <= child.open.start && child.close.end <= p.close.start,
                "crossing style pairs {p:?} {child:?}"
            );
            self.pairs.get_mut(&child).unwrap().parent = Some(p);
            self.gap(task.gap, child.open.end, p);
            task.gap = child.close.start;
            self.roots.remove(&key);
            return false;
        }
        self.gap(task.gap, p.close.start, p);
        self.roots.insert(p.open.start, p);
        self.markers.insert(p.open.start, p.open.end);
        self.markers.insert(p.close.start, p.close.end);
        true
    }
    fn at(&self, at: usize, limit: usize, authoritative: bool) -> Snapshot {
        let inner = self
            .boundaries
            .range(..=at)
            .next_back()
            .and_then(|(_, &p)| p);
        let counts = inner.map_or(Counts::default(), |p| self.pairs[&p].counts);
        let marker_end = self
            .markers
            .range(..=at)
            .next_back()
            .and_then(|(_, &end)| (at < end).then_some(end));
        let mut next = self
            .boundaries
            .range((Excluded(at), Unbounded))
            .next()
            .map_or(limit, |(&at, _)| at.min(limit));
        if let Some(end) = marker_end {
            next = next.min(end);
        }
        if let Some((&start, _)) = self.markers.range((Excluded(at), Unbounded)).next() {
            next = next.min(start);
        }
        Snapshot {
            inner,
            next,
            style: counts.style,
            emphasis: counts.emphasis,
            strong: counts.strong,
            hidden: marker_end.is_some(),
            authoritative,
        }
    }
    fn previous_boundary(&self, at: usize, lower: usize) -> usize {
        let boundary = self
            .boundaries
            .range(..=at)
            .next_back()
            .map_or(lower, |(&at, _)| at);
        let marker = self
            .markers
            .range(..=at)
            .next_back()
            .map_or(lower, |(&start, &end)| if at < end { start } else { end });
        lower.max(boundary).max(marker)
    }
    fn undo_stats(&self) -> crate::rewind::UndoStats {
        self.pairs
            .undo_stats()
            .plus(self.roots.undo_stats())
            .plus(self.boundaries.undo_stats())
            .plus(self.markers.undo_stats())
    }
    fn index_peak(&self) -> usize {
        self.pairs.peak_entries()
            + self.roots.peak_entries()
            + self.boundaries.peak_entries()
            + self.markers.peak_entries()
    }
    fn begin_line(&mut self) {
        self.pairs.begin();
        self.roots.begin();
        self.boundaries.begin();
        self.markers.begin();
    }
    fn commit_line(&mut self) -> u64 {
        self.pairs.commit() + self.roots.commit() + self.boundaries.commit() + self.markers.commit()
    }
    fn rollback_line(&mut self) -> u64 {
        self.pairs.rollback()
            + self.roots.rollback()
            + self.boundaries.rollback()
            + self.markers.rollback()
    }
    fn entries(&self) -> usize {
        self.pairs.len() + self.roots.len() + self.boundaries.len() + self.markers.len()
    }
    fn retire_one(&mut self) -> bool {
        if !self.pairs.is_empty() {
            self.pairs.pop_first();
        } else if !self.roots.is_empty() {
            self.roots.pop_first();
        } else if !self.boundaries.is_empty() {
            self.boundaries.pop_first();
        } else if !self.markers.is_empty() {
            self.markers.pop_first();
        } else {
            return true;
        }
        false
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Snapshot {
    pub inner: Option<StylePairId>,
    pub next: usize,
    style: Style,
    emphasis: usize,
    strong: usize,
    hidden: bool,
    pub(crate) authoritative: bool,
}
impl Snapshot {
    pub(crate) fn apply(self, mut mark: crate::annotations::Mark) -> crate::annotations::Mark {
        if self.authoritative {
            mark.style.0 &= !(Style::EMPHASIS.0 | Style::STRONG.0 | Style::STRIKE.0);
            if mark.delimiter_hidden && !mark.base_hidden {
                mark.style.0 &= !Style::HIDDEN.0;
            }
            mark.style = mark.style.union(self.style);
            if self.hidden {
                mark.style = mark.style.union(Style::HIDDEN);
            }
            mark.emphasis_depth = self.emphasis;
            mark.strong_depth = self.strong;
        }
        mark
    }
}

#[derive(Default)]
pub(crate) struct Styles {
    generations: BTreeMap<usize, u64>,
    pub(crate) shape: crate::DisplayShapeEffects,
    scopes: BTreeMap<usize, Scope>,
    conditional_scopes: BTreeSet<usize>,
    forest: Forest,
    pub(crate) first_use: Option<usize>,
    checkpoint: Option<(usize, Span, usize, usize)>,
    event_entries: usize,
    event_peak: usize,
    label_entries: usize,
    label_peak: usize,
    scope_undo_peak: crate::rewind::UndoStats,
}
impl Styles {
    pub(crate) fn confirmation_stats(
        &self,
    ) -> (usize, usize, usize, usize, crate::rewind::UndoStats, usize) {
        let active =
            self.checkpoint
                .map_or(crate::rewind::UndoStats::default(), |(key, _, _, _)| {
                    let s = &self.scopes[&key];
                    s.events.undo_stats().plus(s.labels.undo_stats())
                });
        (
            self.event_entries,
            self.event_peak,
            self.forest.entries() + self.label_entries,
            self.forest.index_peak() + self.label_peak,
            self.forest
                .undo_stats()
                .plus(self.scope_undo_peak.max(active)),
            self.scopes.len(),
        )
    }

    pub(crate) fn begin_line(&mut self, key: usize) {
        self.shape.emit(crate::DisplayShapeEffectKind::Begin(
            crate::DisplayShapeDomain::Styles { scope: key },
        ));
        assert!(self.checkpoint.is_none());
        let scope = self.scopes.get_mut(&key).unwrap();
        self.checkpoint = Some((key, scope.source, scope.conditional, scope.runs));
        scope.events.begin();
        scope.labels.begin();
        self.forest.begin_line();
    }
    pub(crate) fn commit_line(&mut self) -> u64 {
        let Some((key, _, _, _)) = self.checkpoint.take() else {
            return 0;
        };
        self.shape.emit(crate::DisplayShapeEffectKind::Commit(
            crate::DisplayShapeDomain::Styles { scope: key },
        ));
        let scope = self.scopes.get_mut(&key).unwrap();
        self.scope_undo_peak = self
            .scope_undo_peak
            .max(scope.events.undo_stats().plus(scope.labels.undo_stats()));
        scope.events.commit() + scope.labels.commit() + self.forest.commit_line()
    }
    pub(crate) fn rollback_line(&mut self) -> (Span, u64) {
        let (key, source, conditional, runs) = self.checkpoint.take().unwrap();
        self.shape.emit(crate::DisplayShapeEffectKind::Rollback(
            crate::DisplayShapeDomain::Styles { scope: key },
        ));
        if self.shape.enabled() {
            *self.generations.entry(key).or_default() += 1;
        }
        let scope = self.scopes.get_mut(&key).unwrap();
        let changed = scope.source;
        self.scope_undo_peak = self
            .scope_undo_peak
            .max(scope.events.undo_stats().plus(scope.labels.undo_stats()));
        self.event_entries -= scope.events.len();
        self.label_entries -= scope.labels.len();
        let work = scope.events.rollback() + scope.labels.rollback() + self.forest.rollback_line();
        self.event_entries += scope.events.len();
        self.label_entries += scope.labels.len();
        scope.source = source;
        scope.conditional = conditional;
        scope.runs = runs;
        if conditional == 0 || runs == 0 {
            self.conditional_scopes.remove(&key);
        } else {
            self.conditional_scopes.insert(key);
        }
        (changed, work)
    }

    fn record_shape(&mut self, scope: usize, event: crate::DisplayShapeStyleEvent) {
        if !self.shape.enabled() {
            return;
        }
        let generation = self.generations.entry(scope).or_default();
        *generation = generation
            .checked_add(1)
            .expect("style generation exhausted");
        self.shape.emit(crate::DisplayShapeEffectKind::ScopeEvent {
            scope,
            generation: *generation,
            event,
        });
    }
    pub(crate) fn shape_generation_entries(&self) -> usize {
        self.generations.len()
    }
    pub(crate) fn shape_generation(&self, key: usize) -> u64 {
        self.generations.get(&key).copied().unwrap_or(0)
    }
    pub(crate) fn scope_end(&self, key: usize) -> usize {
        self.scopes[&key].source.end
    }
    pub(crate) fn conditional_source(&self, key: usize) -> Option<Span> {
        let scope = self.scopes.get(&key)?;
        (scope.conditional != 0 && scope.runs != 0).then_some(scope.source)
    }
    pub(crate) fn touch(&mut self, key: usize, span: Span, work: &mut Work) {
        let scope = self.scopes.entry(key).or_insert_with(|| Scope {
            source: Span::new(key, span.end),
            ..Scope::default()
        });
        let old_end = scope.source.end;
        scope.source.end = scope.source.end.max(span.end);
        if scope.source.end != old_end
            || scope.source.end == span.end && old_end == span.end && scope.events.is_empty()
        {
            self.shape.emit(crate::DisplayShapeEffectKind::ScopeExtent {
                scope: key,
                source: scope.source,
            });
        }
        work.style_scope_steps += 1;
    }
    pub(crate) fn enter(&mut self, key: usize, open: usize, work: &mut Work) {
        let scope = self.scopes.get_mut(&key).unwrap();
        let event = (open, 0, 0);
        let previous = scope.events.insert(
            event,
            Event::Enter {
                boundary: Boundary::Transparent,
            },
        );
        self.event_entries += usize::from(previous.is_none());
        self.event_peak = self.event_peak.max(self.event_entries);
        self.label_entries += usize::from(scope.labels.insert(open, (event, None)).is_none());
        self.label_peak = self.label_peak.max(self.label_entries);
        work.style_events += 1;
        self.record_shape(key, crate::DisplayShapeStyleEvent::Enter { at: open });
    }
    pub(crate) fn leave(&mut self, key: usize, open: usize, end: usize, work: &mut Work) {
        let scope = self.scopes.get_mut(&key).unwrap();
        self.event_entries += usize::from(
            scope
                .events
                .insert((end, 2, usize::MAX - open), Event::Leave)
                .is_none(),
        );
        self.event_peak = self.event_peak.max(self.event_entries);
        work.style_events += 1;
        self.record_shape(key, crate::DisplayShapeStyleEvent::Leave { open, end });
    }
    pub(crate) fn run(
        &mut self,
        key: usize,
        run: DelimiterRun,
        unless: Option<LinkId>,
        work: &mut Work,
    ) -> Option<Span> {
        let scope = self.scopes.get_mut(&key).unwrap();
        let was = scope.conditional != 0 && scope.runs != 0;
        let previous = scope
            .events
            .insert((run.source.start, 1, 0), Event::Run { run, unless });
        self.event_entries += usize::from(previous.is_none());
        self.event_peak = self.event_peak.max(self.event_entries);
        if let Some(Event::Run { unless: old, .. }) = previous {
            scope.conditional -= usize::from(old.is_some());
        } else {
            scope.runs += 1;
        }
        scope.conditional += usize::from(unless.is_some());
        let active = scope.conditional != 0 && scope.runs != 0;
        if active {
            self.conditional_scopes.insert(key);
            self.first_use = Some(self.first_use.map_or(key, |old| old.min(key)));
        }
        work.style_events += 1;
        let result = (!was && active).then_some(scope.source);
        self.record_shape(
            key,
            crate::DisplayShapeStyleEvent::Run {
                source: run.source,
                kind: if run.kind < 6 {
                    crate::DisplayDelimiterKind::Asterisk
                } else if run.kind < 12 {
                    crate::DisplayDelimiterKind::Underscore
                } else {
                    crate::DisplayDelimiterKind::Tilde
                },
                can_open: run.can_open,
                can_close: run.can_close,
                unless,
            },
        );
        result
    }
    pub(crate) fn bind(
        &mut self,
        open: usize,
        id: LinkId,
        fixed: Option<bool>,
        work: &mut Work,
    ) -> Option<Span> {
        let (&scope_key, scope) = self.scopes.range_mut(..=open).next_back()?;
        let (key, owner) = scope.labels.get_mut(&open)?;
        if let Some(owner) = *owner {
            assert_eq!(
                owner, id,
                "one syntax opener must reuse its candidate LinkId"
            );
        }
        *owner = Some(id);
        let Some(Event::Enter { boundary }) = scope.events.get_mut(key) else {
            unreachable!()
        };
        let was_conditional = scope.conditional != 0 && scope.runs != 0;
        scope.conditional -= usize::from(boundary.conditional());
        *boundary = fixed.map_or(Boundary::Link(id), Boundary::Fixed);
        scope.conditional += usize::from(boundary.conditional());
        if scope.conditional != 0 && scope.runs != 0 {
            self.conditional_scopes.insert(scope_key);
            self.first_use = Some(
                self.first_use
                    .map_or(scope.source.start, |old| old.min(scope.source.start)),
            );
        } else {
            self.conditional_scopes.remove(&scope_key);
        }
        work.style_scope_steps += 1;
        let result = (was_conditional || (scope.conditional != 0 && scope.runs != 0))
            .then_some(scope.source);
        self.record_shape(
            scope_key,
            crate::DisplayShapeStyleEvent::Bind {
                open,
                boundary: fixed.map_or(
                    crate::DisplayShapeBoundary::Link(id),
                    crate::DisplayShapeBoundary::Fixed,
                ),
            },
        );
        result
    }
    pub(crate) fn conditional_scope_after(&self, span: Span, last: Option<usize>) -> Option<usize> {
        let key = if let Some(last) = last {
            self.conditional_scopes
                .range((Excluded(last), Unbounded))
                .next()
                .copied()
        } else {
            self.conditional_scopes
                .range(..=span.start)
                .next_back()
                .copied()
                .filter(|key| self.scopes[key].source.end > span.start)
                .or_else(|| self.conditional_scopes.range(span.start..).next().copied())
        }?;
        (key < span.end).then_some(key)
    }
    pub(crate) fn pair(&mut self, pair: StylePairId, work: &mut Work) {
        let key = self
            .scopes
            .range(..=pair.open.start)
            .next_back()
            .map_or(pair.open.start, |(&k, _)| k);
        self.record_shape(key, crate::DisplayShapeStyleEvent::Pair(pair));
        let mut task = Insert::new(pair);
        loop {
            work.style_index_steps += 1;
            if self.forest.insert_step(&mut task) {
                break;
            }
        }
    }
    pub(crate) fn fixed_pair(&mut self, pair: StylePairId, work: &mut Work) {
        self.pair(pair, work);
        if let Some((_, scope)) = self.scopes.range_mut(..=pair.open.start).next_back() {
            self.event_entries += usize::from(
                scope
                    .events
                    .insert((pair.open.start, 1, 0), Event::Pair(pair))
                    .is_none(),
            );
            self.event_peak = self.event_peak.max(self.event_entries);
            work.style_events += 1;
        }
    }
    fn scope(&self, at: usize) -> Option<(usize, &Scope)> {
        self.scopes
            .range(..=at)
            .next_back()
            .and_then(|(&key, scope)| (at < scope.source.end).then_some((key, scope)))
    }
    pub(crate) fn slug_scope(&self, at: usize) -> Option<(usize, Span)> {
        self.scope(at).map(|(key, scope)| (key, scope.source))
    }
    pub(crate) fn slug_scope_extent(&self, key: usize) -> Option<Span> {
        self.scopes.get(&key).map(|scope| scope.source)
    }
    pub(crate) fn conditional(&self, at: usize) -> Option<usize> {
        self.scope(at)
            .and_then(|(key, scope)| (scope.conditional != 0 && scope.runs != 0).then_some(key))
    }
    pub(crate) fn unconditional(&self, at: usize, end: usize) -> Snapshot {
        let limit = self.scope(at).map_or_else(
            || {
                self.scopes
                    .range((Excluded(at), Unbounded))
                    .next()
                    .map_or(end, |(&key, _)| key.min(end))
            },
            |(_, s)| s.source.end.min(end),
        );
        self.forest.at(at, limit, false)
    }
    fn previous_scope_boundary(&self, at: usize, lower: usize) -> usize {
        self.scopes
            .range(..=at)
            .next_back()
            .map_or(lower, |(&start, scope)| {
                lower.max(if at < scope.source.end {
                    start
                } else {
                    scope.source.end
                })
            })
    }
}

/// One ordered range walk for the unconditional window. Tree seeks happen
/// only at construction; subsequent pair boundaries are visited once.
pub(crate) struct Window<'a> {
    pub(crate) steps: u64,
    boundaries:
        std::iter::Peekable<std::collections::btree_map::Range<'a, usize, Option<StylePairId>>>,
    inner: Option<StylePairId>,
    conditional: std::iter::Peekable<std::collections::btree_set::Range<'a, usize>>,
    current: Option<Span>,
    scopes: &'a BTreeMap<usize, Scope>,
}
impl Styles {
    pub(crate) fn window(&self, start: usize) -> Window<'_> {
        Window {
            steps: 4,
            boundaries: self
                .forest
                .boundaries
                .range((Excluded(start), Unbounded))
                .peekable(),
            inner: self
                .forest
                .boundaries
                .range(..=start)
                .next_back()
                .and_then(|(_, p)| *p),
            conditional: self.conditional_scopes.range(start..).peekable(),
            current: self
                .conditional_scopes
                .range(..start)
                .next_back()
                .map(|key| self.scopes[key].source)
                .filter(|s| s.end > start),
            scopes: &self.scopes,
        }
    }
}
impl Window<'_> {
    pub(crate) fn snapshot(&mut self, at: usize, end: usize) -> Option<Snapshot> {
        self.steps += 1;
        if self.current.is_some_and(|s| s.end <= at) {
            self.current = None;
        }
        while self.conditional.peek().is_some_and(|key| **key <= at) {
            self.steps += 1;
            let key = self.conditional.next().unwrap();
            let source = self.scopes[key].source;
            if source.end > at {
                self.current = Some(source);
            }
        }
        while self.boundaries.peek().is_some_and(|(key, _)| **key <= at) {
            self.steps += 1;
            self.inner = *self.boundaries.next().unwrap().1;
        }
        if self.current.is_some() {
            return None;
        }
        let next = self
            .boundaries
            .peek()
            .map_or(end, |(key, _)| (**key).min(end));
        let next = self
            .conditional
            .peek()
            .map_or(next, |key| (**key).min(next));
        Some(Snapshot {
            inner: self.inner,
            next,
            style: Style::default(),
            emphasis: 0,
            strong: 0,
            hidden: false,
            authoritative: false,
        })
    }
}

// Fixed pages avoid a frame-sized reallocation/copy of a deep query stack.
const PAGE: usize = 64;
pub(crate) struct Pages<T: Copy> {
    pages: BTreeMap<usize, Box<[Option<T>; PAGE]>>,
    len: usize,
}
impl<T: Copy> Default for Pages<T> {
    fn default() -> Self {
        Self {
            pages: BTreeMap::new(),
            len: 0,
        }
    }
}
impl<T: Copy> Pages<T> {
    pub(crate) fn push(&mut self, value: T) {
        self.pages
            .entry(self.len / PAGE)
            .or_insert_with(|| Box::new([None; PAGE]))[self.len % PAGE] = Some(value);
        self.len += 1;
    }
    fn get(&self, at: usize) -> T {
        self.pages[&(at / PAGE)][at % PAGE].unwrap()
    }
    fn set(&mut self, at: usize, value: T) {
        self.pages.get_mut(&(at / PAGE)).unwrap()[at % PAGE] = Some(value);
    }
    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let result = self.pages.get_mut(&(self.len / PAGE)).unwrap()[self.len % PAGE].take();
        if self.len.is_multiple_of(PAGE) {
            self.pages.remove(&(self.len / PAGE));
        }
        result
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(crate) fn last(&self) -> Option<T> {
        self.len.checked_sub(1).map(|i| self.get(i))
    }
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.pages.len() * std::mem::size_of::<[Option<T>; PAGE]>()
    }
}

#[derive(Clone, Copy)]
struct Opener {
    source: Span,
    kind: usize,
    previous: Option<usize>,
}
#[derive(Clone, Copy)]
struct RunTask {
    run: DelimiterRun,
    remaining: Span,
    bucket: usize,
    best: Option<usize>,
    phase: u8,
}
impl RunTask {
    fn new(run: DelimiterRun) -> Self {
        Self {
            run,
            remaining: run.source,
            bucket: if run.kind >= 12 {
                run.kind
            } else {
                run.kind / 6 * 6
            },
            best: None,
            phase: 0,
        }
    }
}
enum Phase {
    Event,
    Guard {
        task: Box<LinkActivity>,
        run: Option<DelimiterRun>,
    },
    Exit(usize),
    Run(RunTask),
    Insert {
        insert: Insert,
        next: Option<RunTask>,
    },
    Aggregate(Option<StylePairId>),
    Complete,
}
struct Recovery {
    last: Option<EventKey>,
    heads: [Option<usize>; 14],
    stack: Pages<Opener>,
    floors: Pages<(usize, bool)>,
    floor: usize,
    forest: Forest,
    phase: Phase,
    aggregate_first: Option<StylePairId>,
    aggregate_end: usize,
    more: bool,
    revision: u64,
    changes: BTreeMap<(u64, usize), Span>,
    marker_revision: BTreeMap<usize, u64>,
}
impl Default for Recovery {
    fn default() -> Self {
        Self {
            last: None,
            heads: [None; 14],
            stack: Pages::default(),
            floors: Pages::default(),
            floor: 0,
            forest: Forest::default(),
            phase: Phase::Event,
            aggregate_first: None,
            aggregate_end: 0,
            more: false,
            revision: 0,
            changes: BTreeMap::new(),
            marker_revision: BTreeMap::new(),
        }
    }
}
impl Recovery {
    fn pop(&mut self) {
        let d = self.stack.pop().unwrap();
        self.heads[d.kind] = d.previous;
    }
    fn enter(&mut self, active: bool) {
        self.floors.push((self.floor, active));
        if active {
            self.floor = self.stack.len;
        }
    }
    fn step(
        &mut self,
        doc: &Document,
        key: usize,
        resolution: &mut Resolution,
        budget: &mut Budget,
        work: &mut StyleResolutionWork,
    ) -> Result<bool, LayoutError> {
        loop {
            if matches!(self.phase, Phase::Complete) {
                return Ok(true);
            }
            if let Phase::Guard { task, .. } = &mut self.phase {
                task.rebind_shared_revision(doc);
                let Some(active) = task.advance_in(doc, resolution, budget, 0)? else {
                    return Ok(false);
                };
                if !charge(budget) {
                    return Ok(false);
                }
                let next = match std::mem::replace(&mut self.phase, Phase::Event) {
                    Phase::Guard { run, .. } => run,
                    _ => unreachable!(),
                };
                if let Some(run) = next {
                    if active != LinkDisposition::Active {
                        self.phase = Phase::Run(RunTask::new(run));
                    }
                } else {
                    self.enter(active == LinkDisposition::Active);
                }
                work.scope_steps += 1;
                continue;
            }
            if !charge(budget) {
                return Ok(false);
            }
            match std::mem::replace(&mut self.phase, Phase::Event) {
                Phase::Event => {
                    let scope = &doc.inline_styles.scopes[&key];
                    let next = if let Some(last) = self.last {
                        scope.events.range((Excluded(last), Unbounded)).next()
                    } else {
                        scope.events.first_key_value()
                    };
                    if let Some((&at, &event)) = next {
                        self.last = Some(at);
                        work.events += 1;
                        match event {
                            Event::Pair(pair) => {
                                self.phase = Phase::Insert {
                                    insert: Insert::new(pair),
                                    next: None,
                                }
                            }
                            Event::Run { run, unless } => {
                                self.phase = unless.map_or_else(
                                    || Phase::Run(RunTask::new(run)),
                                    |id| Phase::Guard {
                                        task: Box::new(doc.link_activity(id).unwrap()),
                                        run: Some(run),
                                    },
                                )
                            }
                            Event::Enter { boundary } => match boundary {
                                Boundary::Transparent | Boundary::Fixed(false) => self.enter(false),
                                Boundary::Fixed(true) => self.enter(true),
                                Boundary::Link(id) => {
                                    self.phase = Phase::Guard {
                                        task: Box::new(doc.link_activity(id).unwrap()),
                                        run: None,
                                    }
                                }
                            },
                            Event::Leave => {
                                let (old, active) = self.floors.pop().unwrap();
                                let floor = self.floor;
                                self.floor = old;
                                if active {
                                    self.phase = Phase::Exit(floor);
                                }
                            }
                        }
                    } else {
                        self.more = false;
                        self.phase = Phase::Aggregate(None);
                    }
                }
                Phase::Exit(floor) => {
                    if self.stack.len > floor {
                        self.pop();
                        work.pops += 1;
                        self.phase = Phase::Exit(floor);
                    }
                }
                Phase::Run(mut task) => {
                    let run = task.run;
                    if task.phase == 0 && run.can_close && !task.remaining.is_empty() {
                        let end = if run.kind >= 12 {
                            run.kind + 1
                        } else {
                            run.kind / 6 * 6 + 6
                        };
                        if task.bucket < end {
                            work.head_reads += 1;
                            if run.eligible(task.bucket)
                                && let Some(i) = self.heads[task.bucket]
                                && i >= self.floor
                            {
                                task.best = task.best.max(Some(i));
                            }
                            task.bucket += 1;
                            self.phase = Phase::Run(task);
                            continue;
                        }
                        if task.best.is_some() {
                            task.phase = 1;
                            self.phase = Phase::Run(task);
                            continue;
                        }
                    }
                    if task.phase == 1 {
                        let i = task.best.unwrap();
                        if self.stack.len > i + 1 {
                            self.pop();
                            work.pops += 1;
                            self.phase = Phase::Run(task);
                            continue;
                        }
                        let mut opener = self.stack.get(i);
                        let used = if run.kind >= 12 {
                            task.remaining.len()
                        } else {
                            2.min(opener.source.len()).min(task.remaining.len())
                        };
                        let pair = StylePairId {
                            open: Span::new(opener.source.end - used, opener.source.end),
                            close: Span::new(task.remaining.start, task.remaining.start + used),
                            kind: if run.kind >= 12 {
                                InlineStyleKind::Strike
                            } else if used == 2 {
                                InlineStyleKind::Strong
                            } else {
                                InlineStyleKind::Emphasis
                            },
                        };
                        opener.source.end -= used;
                        if opener.source.is_empty() {
                            self.pop();
                            work.pops += 1;
                        } else {
                            self.stack.set(i, opener);
                        }
                        task.remaining.start += used;
                        task.phase = 0;
                        task.best = None;
                        task.bucket = if run.kind >= 12 {
                            run.kind
                        } else {
                            run.kind / 6 * 6
                        };
                        self.phase = Phase::Insert {
                            insert: Insert::new(pair),
                            next: Some(task),
                        };
                        work.pairs += 1;
                        continue;
                    }
                    if run.can_open && !task.remaining.is_empty() {
                        let i = self.stack.len;
                        self.stack.push(Opener {
                            source: task.remaining,
                            kind: run.kind,
                            previous: self.heads[run.kind],
                        });
                        self.heads[run.kind] = Some(i);
                        work.pushes += 1;
                    }
                }
                Phase::Insert { mut insert, next } => {
                    work.index_steps += 1;
                    self.phase = if self.forest.insert_step(&mut insert) {
                        if insert.started {
                            let pair = insert.pair;
                            self.aggregate_first =
                                Some(self.aggregate_first.map_or(pair, |old| old.min(pair)));
                            self.aggregate_end = self.aggregate_end.max(pair.close.end);
                            for span in [pair.open, pair.close] {
                                if let Some(old) =
                                    self.marker_revision.insert(span.start, doc.revision())
                                {
                                    self.changes.remove(&(old, span.start));
                                }
                                self.changes.insert((doc.revision(), span.start), span);
                            }
                        }
                        next.map_or(Phase::Event, Phase::Run)
                    } else {
                        Phase::Insert { insert, next }
                    };
                }
                Phase::Aggregate(last) => {
                    let next = if let Some(last) = last {
                        self.forest.pairs.range((Excluded(last), Unbounded)).next()
                    } else {
                        self.aggregate_first
                            .and_then(|first| self.forest.pairs.range(first..).next())
                    };
                    if let Some((&id, &node)) = next
                        && id.open.start < self.aggregate_end
                    {
                        let counts = node
                            .parent
                            .map_or(Counts::default(), |p| self.forest.pairs[&p].counts)
                            .with(id.kind);
                        self.forest.pairs.get_mut(&id).unwrap().counts = counts;
                        self.phase = Phase::Aggregate(Some(id));
                        work.index_steps += 1;
                    } else {
                        self.aggregate_first = None;
                        self.aggregate_end = 0;
                        if self.more {
                            self.more = false;
                            self.phase = Phase::Event;
                        } else {
                            self.revision = doc.revision();
                            self.phase = Phase::Complete;
                        }
                    }
                }
                Phase::Complete | Phase::Guard { .. } => unreachable!(),
            }
        }
    }
    fn entries(&self) -> usize {
        1 + self.stack.len
            + self.floors.len
            + self.forest.entries()
            + self.changes.len()
            + self.marker_revision.len()
            + match &self.phase {
                Phase::Guard { task, .. } => task.continuation_entries(),
                _ => 0,
            }
    }
    fn capacity_bytes(&self) -> usize {
        self.stack.capacity_bytes()
            + self.floors.capacity_bytes()
            + self.changes.len() * std::mem::size_of::<((u64, usize), Span)>()
            + self.marker_revision.len() * std::mem::size_of::<(usize, u64)>()
            + self.forest.pairs.len() * std::mem::size_of::<(StylePairId, Pair)>()
            + self.forest.roots.len() * std::mem::size_of::<(usize, StylePairId)>()
            + self.forest.boundaries.len() * std::mem::size_of::<(usize, Option<StylePairId>)>()
            + self.forest.markers.len() * std::mem::size_of::<(usize, usize)>()
            + match &self.phase {
                Phase::Guard { task, .. } => {
                    std::mem::size_of_val(&**task) + task.continuation_capacity()
                }
                _ => 0,
            }
    }
    fn retire_one(&mut self) -> bool {
        if let Phase::Guard { task, .. } = &mut self.phase
            && task.continuation_entries() != 0
        {
            task.retire_one();
            return false;
        }
        if self.stack.pop().is_some() || self.floors.pop().is_some() {
            return false;
        }
        if self.changes.pop_first().is_some() || self.marker_revision.pop_first().is_some() {
            return false;
        }
        self.forest.retire_one()
    }
}

/// Actual conditional metadata work. These operations share the caller's
/// Budget with link guards, text, layout and dependency capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleResolutionWork {
    pub events: u64,
    pub head_reads: u64,
    pub pushes: u64,
    pub pops: u64,
    pub pairs: u64,
    pub scope_steps: u64,
    pub index_steps: u64,
    pub retired_entries: u64,
}
#[derive(Default)]
pub(crate) struct Resolver {
    scopes: BTreeMap<usize, Recovery>,
    live: usize,
    pub work: StyleResolutionWork,
}
fn charge(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    })
}
impl Resolver {
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.scopes
            .values()
            .map(|s| std::mem::size_of::<(usize, Recovery)>() + s.capacity_bytes())
            .sum()
    }
    pub(crate) fn append_style(
        &mut self,
        doc: &Document,
        key: usize,
        event: crate::DisplayShapeStyleEvent,
    ) -> bool {
        let Some(state) = self.scopes.get_mut(&key) else {
            return true;
        };
        if state.revision == doc.revision() && matches!(state.phase, Phase::Complete) {
            return true;
        }
        use crate::DisplayShapeStyleEvent as S;
        let at = match event {
            S::Run { source, .. } => (source.start, 1, 0),
            S::Enter { at } => (at, 0, 0),
            S::Leave { open, end } => (end, 2, usize::MAX - open),
            S::Bind { .. } => return false,
            S::Pair(pair) => {
                if !matches!(
                    doc.inline_styles
                        .scopes
                        .get(&key)
                        .and_then(|s| s.events.get(&(pair.open.start, 1, 0))),
                    Some(Event::Pair(_))
                ) {
                    return true;
                }
                (pair.open.start, 1, 0)
            }
        };
        if state.last.is_some_and(|last| at <= last) {
            return false;
        }
        if matches!(state.phase, Phase::Complete) {
            state.phase = Phase::Event;
        } else {
            state.more = true;
        }
        true
    }
    pub(crate) fn changed_marker(
        &self,
        key: usize,
        since: u64,
        last: Option<(u64, usize)>,
    ) -> Option<((u64, usize), Span)> {
        let state = self.scopes.get(&key)?;
        let low = last.unwrap_or((since, usize::MAX));
        state
            .changes
            .range((Excluded(low), Unbounded))
            .next()
            .map(|(&k, &v)| (k, v))
    }
    pub(crate) fn entries(&self) -> usize {
        self.live
    }
    pub(crate) fn prepare(
        &mut self,
        doc: &Document,
        at: usize,
        resolution: &mut Resolution,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if budget.remaining().queries == 0 || budget.remaining().objects == 0 {
            return Ok(false);
        }
        let Some(key) = doc.inline_styles.conditional(at) else {
            return Ok(true);
        };
        if let std::collections::btree_map::Entry::Vacant(entry) = self.scopes.entry(key) {
            if !charge(budget) {
                return Ok(false);
            }
            entry.insert(Recovery::default());
            self.live += 1;
            self.work.scope_steps += 1;
        }
        let state = self.scopes.get_mut(&key).unwrap();
        let before = state.entries();
        let result = state.step(doc, key, resolution, budget, &mut self.work);
        self.live = self.live - before + state.entries();
        result
    }
    pub(crate) fn snapshot(&self, doc: &Document, at: usize, end: usize) -> Snapshot {
        if let Some(key) = doc.inline_styles.conditional(at) {
            let state = &self.scopes[&key];
            debug_assert!(matches!(state.phase, Phase::Complete));
            state
                .forest
                .at(at, doc.inline_styles.scopes[&key].source.end.min(end), true)
        } else {
            doc.inline_styles.unconditional(at, end)
        }
    }
    fn previous_boundary(&self, doc: &Document, at: usize, lower: usize) -> usize {
        let lower = doc.inline_styles.previous_scope_boundary(at, lower);
        if let Some(key) = doc.inline_styles.conditional(at) {
            self.scopes[&key].forest.previous_boundary(at, lower)
        } else {
            doc.inline_styles.forest.previous_boundary(at, lower)
        }
    }
    pub(crate) fn retire(&mut self, budget: &mut Budget) -> Completion {
        while let Some((&key, state)) = self.scopes.first_key_value() {
            let _ = state;
            if !charge(budget) {
                return Completion::BudgetExhausted;
            }
            let state = self.scopes.get_mut(&key).unwrap();
            let before = state.entries();
            let complete = state.retire_one();
            self.live = self.live - before + state.entries();
            if complete {
                self.scopes.remove(&key);
                self.live -= 1;
            }
            self.work.retired_entries += 1;
        }
        Completion::Covered
    }
}
impl Resolution {
    pub(crate) fn prepare_styles(
        &mut self,
        doc: &Document,
        at: usize,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        let mut styles = std::mem::take(&mut self.styles);
        let result = styles.prepare(doc, at, self, budget);
        self.styles = styles;
        result
    }
    pub(crate) fn style_snapshot(&mut self, doc: &Document, at: usize, end: usize) -> Snapshot {
        assert!(
            self.prepare_styles(doc, at, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
        );
        self.styles.snapshot(doc, at, end)
    }
}

pub enum StylePathStep {
    Pair(StylePairId),
    BudgetExhausted,
    Complete,
}
/// Ordered outer-to-inner path. Construct without evaluating any metadata;
/// keep it and the shared resolver between frames. No source is tokenized.
pub struct StylePath {
    document: DocumentId,
    revision: u64,
    at: usize,
    inner: Option<StylePairId>,
    path: Pages<StylePairId>,
    phase: u8,
}
impl Document {
    pub fn style_path(&self, at: usize) -> Option<StylePath> {
        (at < self.source.len() && self.source.is_char_boundary(at)).then(|| StylePath {
            document: self.id(),
            revision: self.revision(),
            at,
            inner: None,
            path: Pages::default(),
            phase: 0,
        })
    }
}
impl StylePath {
    pub fn advance(
        &mut self,
        doc: &Document,
        resolver: &mut crate::LinkResolver,
        budget: &mut Budget,
    ) -> Result<StylePathStep, LayoutError> {
        if self.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != doc.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        resolver.check(doc)?;
        loop {
            if self.phase == 3 {
                return Ok(StylePathStep::Complete);
            }
            if self.phase == 0 {
                if !resolver.resolution.prepare_styles(doc, self.at, budget)? {
                    return Ok(StylePathStep::BudgetExhausted);
                }
                if !charge(budget) {
                    return Ok(StylePathStep::BudgetExhausted);
                }
                self.inner = resolver
                    .resolution
                    .styles
                    .snapshot(doc, self.at, doc.source.len())
                    .inner;
                self.phase = 1;
            } else if self.phase == 1 {
                if !charge(budget) {
                    return Ok(StylePathStep::BudgetExhausted);
                }
                if let Some(id) = self.inner {
                    self.path.push(id);
                    let forest = if let Some(key) = doc.inline_styles.conditional(self.at) {
                        &resolver.resolution.styles.scopes[&key].forest
                    } else {
                        &doc.inline_styles.forest
                    };
                    self.inner = forest.pairs[&id].parent;
                } else {
                    self.phase = 2;
                }
            } else {
                if !charge(budget) {
                    return Ok(StylePathStep::BudgetExhausted);
                }
                if let Some(id) = self.path.pop() {
                    return Ok(StylePathStep::Pair(id));
                }
                self.phase = 3;
            }
        }
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.phase = 3;
        while self.path.len != 0 {
            if !charge(budget) {
                return Completion::BudgetExhausted;
            }
            self.path.pop();
        }
        Completion::Covered
    }
}

/// One source-ordered style gap or marker. This is classified metadata, not
/// display text. An authoritative conditional scope may reveal delimiter hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedStyleSpan {
    pub source: Span,
    pub scope: Option<usize>,
    pub scope_generation: u64,
    pub marker_hidden: bool,
    pub authoritative: bool,
    pub style_pair: Option<StylePairId>,
    pub style: Style,
    pub emphasis_depth: usize,
    pub strong_depth: usize,
}
impl ResolvedStyleSpan {
    /// Apply the same conditional style override as the canonical run reader.
    /// Link/alternative ownership is still resolved by the shared link resolver.
    pub fn apply(self, mut mark: crate::DisplayShapeMark) -> crate::DisplayShapeMark {
        if self.authoritative {
            mark.style.0 &= !(Style::EMPHASIS.0 | Style::STRONG.0 | Style::STRIKE.0);
            if mark.delimiter_hidden && !mark.base_hidden {
                mark.style.0 &= !Style::HIDDEN.0;
            }
            mark.style = mark.style.union(self.style);
            if self.marker_hidden {
                mark.style = mark.style.union(Style::HIDDEN);
            }
            mark.emphasis_depth = self.emphasis_depth;
            mark.strong_depth = self.strong_depth;
        }
        mark
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleMarkerStep {
    Span(ResolvedStyleSpan),
    BudgetExhausted,
    Complete,
}

/// Revision-bound cursor; no source bytes, parser, payload copies or private
/// memo. All conditional recovery belongs to the caller's canonical resolver.
pub struct StyleMarkers {
    document: DocumentId,
    revision: u64,
    position: usize,
    end: usize,
    reverse: bool,
}
impl Document {
    pub fn style_markers(&self, source: Span) -> Option<StyleMarkers> {
        self.text(source)?;
        Some(StyleMarkers {
            document: self.id(),
            revision: self.revision(),
            position: source.start,
            end: source.end,
            reverse: false,
        })
    }
    /// Native classified marker/gap spans in reverse source order. This keeps
    /// one snapshot and uses fixed O(log n) predecessor queries; no source
    /// decoding, buffering or separate resolver is involved.
    pub fn style_markers_reverse(&self, source: Span) -> Option<StyleMarkers> {
        let mut cursor = self.style_markers(source)?;
        cursor.position = source.end;
        cursor.end = source.start;
        cursor.reverse = true;
        Some(cursor)
    }
}
impl StyleMarkers {
    pub fn rebind_with(
        &mut self,
        document: &Document,
        resolver: &crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        resolver.retained(document, self.revision)?;
        self.revision = document.revision();
        Ok(())
    }

    pub fn source_position(&self) -> usize {
        self.position
    }
    /// A step pays one query/object for a fixed O(log n) snapshot, in addition
    /// to actual shared scope-recovery D. One-query frames retain recovery;
    /// after it finishes a later frame emits the span. Zero budget does no work.
    pub fn advance(
        &mut self,
        doc: &Document,
        resolver: &mut crate::LinkResolver,
        budget: &mut Budget,
    ) -> Result<StyleMarkerStep, LayoutError> {
        if self.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != doc.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        resolver.check(doc)?;
        if self.position == self.end {
            return Ok(StyleMarkerStep::Complete);
        }
        // A last-byte lookup selects already classified metadata, including
        // inside a UTF-8 scalar; returned boundaries remain native source cuts.
        let at = if self.reverse {
            self.position - 1
        } else {
            self.position
        };
        if !resolver.resolution.prepare_styles(doc, at, budget)? {
            return Ok(StyleMarkerStep::BudgetExhausted);
        }
        if !charge(budget) {
            return Ok(StyleMarkerStep::BudgetExhausted);
        }
        let snapshot = resolver.resolution.style_snapshot(
            doc,
            at,
            if self.reverse {
                self.position
            } else {
                self.end
            },
        );
        let scope = doc.inline_styles.scope(at).map(|(key, _)| key);
        let source = if self.reverse {
            let previous = resolver
                .resolution
                .styles
                .previous_boundary(doc, at, self.end);
            debug_assert!(previous < self.position);
            Span::new(previous, self.position)
        } else {
            let next = snapshot.next.min(self.end);
            debug_assert!(next > self.position);
            Span::new(self.position, next)
        };
        let value = ResolvedStyleSpan {
            source,
            scope,
            scope_generation: scope.map_or(0, |key| doc.inline_styles.shape_generation(key)),
            marker_hidden: snapshot.hidden,
            authoritative: snapshot.authoritative,
            style_pair: snapshot.inner,
            style: snapshot.style,
            emphasis_depth: snapshot.emphasis,
            strong_depth: snapshot.strong,
        };
        self.position = if self.reverse {
            source.start
        } else {
            source.end
        };
        Ok(StyleMarkerStep::Span(value))
    }
}

/// One real classified event per step; no source lexing or pair reconstruction.
#[derive(Default)]
pub(crate) struct ShapeSeed {
    scope: Option<usize>,
    event: Option<EventKey>,
    done: bool,
}
impl ShapeSeed {
    pub(crate) fn conditional_source(&self, styles: &Styles) -> Option<Span> {
        styles.conditional_source(self.scope?)
    }
    pub(crate) fn next(&mut self, styles: &Styles) -> Option<Option<Span>> {
        if self.done {
            return None;
        }
        if self.scope.is_none() {
            self.scope = styles.scopes.first_key_value().map(|(&key, _)| key);
        }
        loop {
            let Some(key) = self.scope else {
                self.done = true;
                return None;
            };
            let scope = &styles.scopes[&key];
            let next = match self.event {
                Some(key) => scope.events.range((Excluded(key), Unbounded)).next(),
                None => scope.events.first_key_value(),
            };
            if let Some((&key, event)) = next {
                self.event = Some(key);
                return Some(if let Event::Run { run, .. } = event {
                    Some(run.source)
                } else {
                    None
                });
            }
            self.scope = styles
                .scopes
                .range((Excluded(key), Unbounded))
                .next()
                .map(|(&key, _)| key);
            self.event = None;
            // The caller pays another metadata step before visiting a new scope.
            if self.scope.is_some() {
                return Some(None);
            }
        }
    }
}
