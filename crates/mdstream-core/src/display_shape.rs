//! Input-side semantic effects for append-only display-property consumers.
//! No Unicode properties or Markdown interpretation are implemented here.
use crate::{
    Block, BlockId, ChangeKind, Document, FootnoteId, FootnoteReferenceId, LinkId, LinkInfo,
    LinkTarget, MathId, Span, Style, StylePairId, SymbolId,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayShapeDomain {
    Annotations,
    Replacements,
    LinkParents,
    FootnoteReferences,
    Styles { scope: usize },
}

/// A producer mark, before canonical conditional/style resolution. Delimiter
/// hints can be undone by an authoritative style snapshot; base hiding cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayShapeMark {
    pub style: Style,
    pub base_hidden: bool,
    pub delimiter_hidden: bool,
    pub emphasis_depth: usize,
    pub strong_depth: usize,
    pub link: Option<LinkId>,
    pub hide_owner: Option<LinkId>,
    pub hide_alternative: Option<LinkId>,
    pub object_syntax: bool,
    pub order: u64,
}

/// Immutable descriptor. Truncating/reusing the parser's display slots cannot
/// change a descriptor already delivered in an Update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayShapeReplacement {
    Entity { utf8: [u8; 8], len: u8 },
    Code { leading: Span, trailing: Span },
    Spaces { columns: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayShapeBoundary {
    Transparent,
    Fixed(bool),
    Link(LinkId),
}

/// Classified ASCII delimiter; `Run.source.len()` is its original marker
/// count, including the rule-of-three residue. No source lexing is required.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayDelimiterKind {
    Asterisk,
    Underscore,
    Tilde,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayShapeStyleEvent {
    Enter {
        at: usize,
    },
    Leave {
        open: usize,
        end: usize,
    },
    Run {
        source: Span,
        kind: DisplayDelimiterKind,
        can_open: bool,
        can_close: bool,
        unless: Option<LinkId>,
    },
    Bind {
        open: usize,
        boundary: DisplayShapeBoundary,
    },
    Pair(StylePairId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayShapeEffectKind {
    SourceAppend(Span),
    Finish,
    Begin(DisplayShapeDomain),
    Commit(DisplayShapeDomain),
    Rollback(DisplayShapeDomain),
    Mark {
        source: Span,
        mark: DisplayShapeMark,
    },
    Replacement {
        source: Span,
        replacement: DisplayShapeReplacement,
    },
    TruncateReplacements {
        entries: usize,
    },
    ScopeExtent {
        scope: usize,
        source: Span,
    },
    ScopeEvent {
        scope: usize,
        generation: u64,
        event: DisplayShapeStyleEvent,
    },
    Link {
        id: LinkId,
        info: LinkInfo,
        target: LinkTarget,
    },
    LinkSource {
        id: LinkId,
        source: Span,
    },
    LinkGate {
        id: LinkId,
        conditional: bool,
        nested: bool,
    },
    LinkParent {
        id: LinkId,
        parent: Option<LinkId>,
    },
    LinkActive {
        id: LinkId,
        active: bool,
    },
    LinkState {
        id: LinkId,
        state: crate::SyntaxState,
    },
    Reference {
        id: SymbolId,
        generation: u64,
    },
    Block {
        id: BlockId,
        block: Block,
    },
    /// A real structural mutation; current block/table/container metadata can
    /// be read by source/ID without decoding the affected body.
    Structure {
        source: Span,
        kind: ChangeKind,
    },
    Math {
        id: MathId,
        math: crate::Math,
        state: Option<crate::SyntaxState>,
    },
    Footnote {
        id: FootnoteId,
    },
    FootnoteReference {
        id: FootnoteReferenceId,
        reference: crate::FootnoteReference,
        state: Option<crate::SyntaxState>,
    },
}
impl DisplayShapeEffectKind {
    fn changes_binding(&self) -> bool {
        matches!(
            self,
            Self::ScopeEvent { .. }
                | Self::Link { .. }
                | Self::LinkGate { .. }
                | Self::LinkParent { .. }
                | Self::LinkActive { .. }
                | Self::Reference { .. }
                | Self::Footnote { .. }
                | Self::FootnoteReference { .. }
                | Self::Math { .. }
                | Self::LinkState { .. }
                | Self::Finish
                | Self::Rollback(_)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayShapeEffect {
    pub sequence: usize,
    pub kind: DisplayShapeEffectKind,
}

#[derive(Default)]
pub(crate) struct ShapeClock {
    sequence: AtomicUsize,
    binding: AtomicUsize,
}
fn increment(value: &AtomicUsize) -> usize {
    value
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .expect("display shape generation exhausted")
        + 1
}

/// Opt-in ordered effects owned by one Update. Consume updates in order, or
/// merge them before consuming. The Document retains no effect history.
#[derive(Clone, Default)]
pub struct DisplayShapeEffects {
    clock: Option<Arc<ShapeClock>>,
    events: Vec<DisplayShapeEffect>,
}
impl std::fmt::Debug for DisplayShapeEffects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayShapeEffects")
            .field("enabled", &self.enabled())
            .field("events", &self.events)
            .finish()
    }
}
impl PartialEq for DisplayShapeEffects {
    fn eq(&self, other: &Self) -> bool {
        self.enabled() == other.enabled() && self.events == other.events
    }
}
impl Eq for DisplayShapeEffects {}
impl DisplayShapeEffects {
    pub fn enabled(&self) -> bool {
        self.clock.is_some()
    }
    pub fn events(&self) -> &[DisplayShapeEffect] {
        &self.events
    }
    pub fn capacity_bytes(&self) -> usize {
        self.events.capacity() * std::mem::size_of::<DisplayShapeEffect>()
    }
    pub(crate) fn enable(&mut self, clock: Arc<ShapeClock>) {
        self.clock = Some(clock);
    }
    pub(crate) fn emit(&mut self, kind: DisplayShapeEffectKind) -> usize {
        let Some(clock) = &self.clock else {
            return 0;
        };
        if kind.changes_binding() {
            increment(&clock.binding);
        }
        let sequence = increment(&clock.sequence);
        self.events.push(DisplayShapeEffect { sequence, kind });
        sequence
    }
    pub(crate) fn drain_into(&mut self, out: &mut Self) {
        out.events.append(&mut self.events);
    }
    pub(crate) fn reset_update(&mut self, clock: &Self) {
        self.events.clear();
        self.clock = clock.clock.clone();
    }
    pub(crate) fn merge(&mut self, next: &mut Self) {
        self.events.append(&mut next.events);
    }
    pub(crate) fn sort(&mut self) {
        self.events.sort_unstable_by_key(|e| e.sequence);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayShapeStats {
    pub emitted: usize,
    /// Nonlocal semantic/presentation binding changes, including footnote
    /// numbering/definitions, active state and EOF. SourceAppend, range marks,
    /// replacement and extent changes are applied separately by the consumer.
    pub binding_epoch: usize,
    /// One current counter per classified scope, never one per revision.
    pub scope_generations: usize,
    /// Retained producer buffers only; caller-owned Updates are separate.
    pub producer_capacity_bytes: usize,
}

impl Document {
    /// Enable before the first nonempty append. This is an incremental input
    /// protocol, not a late subscription that reparses existing Markdown.
    pub fn enable_display_shape_effects(&mut self) -> Result<(), crate::LayoutError> {
        if !self.source.is_empty() || self.finished {
            return Err(crate::LayoutError::InvalidRange);
        }
        if self.shape.enabled() {
            return Ok(());
        }
        let clock = Arc::new(ShapeClock::default());
        for log in [
            &mut self.shape,
            &mut self.annotations.shape,
            &mut self.display.shape,
            &mut self.inline_styles.shape,
            &mut self.link_order.shape,
            &mut self.references.shape,
            &mut self.footnotes.shape,
        ] {
            log.enable(clock.clone());
        }
        Ok(())
    }
    pub fn display_shape_stats(&self) -> DisplayShapeStats {
        let Some(clock) = &self.shape.clock else {
            return DisplayShapeStats::default();
        };
        DisplayShapeStats {
            emitted: clock.sequence.load(Ordering::Relaxed),
            binding_epoch: clock.binding.load(Ordering::Relaxed),
            scope_generations: self.inline_styles.shape_generation_entries(),
            producer_capacity_bytes: [
                &self.shape,
                &self.annotations.shape,
                &self.display.shape,
                &self.inline_styles.shape,
                &self.link_order.shape,
                &self.references.shape,
                &self.footnotes.shape,
            ]
            .iter()
            .map(|log| log.capacity_bytes())
            .sum(),
        }
    }
    pub(crate) fn drain_display_shape(&mut self, update: &mut crate::Update) {
        for log in [
            &mut self.shape,
            &mut self.annotations.shape,
            &mut self.display.shape,
            &mut self.inline_styles.shape,
            &mut self.link_order.shape,
            &mut self.references.shape,
            &mut self.footnotes.shape,
        ] {
            log.drain_into(&mut update.display_shape);
        }
        update.display_shape.sort();
    }
}

/// Work performed by a current-metadata seed. Counts include checkpoint undo
/// gathering, both tree traversals, phase visits and paid retirement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayShapeSeedWork {
    pub steps: usize,
    pub undo_copies: usize,
    pub annotation_nodes: usize,
    pub retired_nodes: usize,
}

/// Current metadata, not an invented Update/event history. Annotation spans
/// address source bytes and may end within a scalar: consumers assign each tag
/// to scalar starts inside the span. Source classification is separately paid.
#[derive(Clone, Copy, Debug)]
pub enum DisplayShapeSeedItem {
    AnnotationCheckpoint,
    Mark {
        source: Span,
        mark: DisplayShapeMark,
        baseline: bool,
    },
    Replacement {
        source: Span,
        replacement: DisplayShapeReplacement,
    },
    Candidate {
        source: Span,
        conditional_scope: Option<Span>,
    },
    Link {
        id: LinkId,
        info: LinkInfo,
        active: bool,
    },
    FootnoteReference {
        id: FootnoteReferenceId,
        source: Span,
    },
}
/// Bounded read-only traversal of one current root namespace. It excludes
/// body-local IDs and carries the live annotation rollback checkpoint.
pub struct DisplayShapeSeed {
    document: crate::DocumentId,
    revision: u64,
    phase: u8,
    at: usize,
    annotations: crate::annotations::ShapeSeed,
    styles: crate::inline_styles::ShapeSeed,
    steps: usize,
}
impl Document {
    /// Current classified conditional-style dependency for one root scope ID.
    /// Pure extent growth does not change its generation or recover any source.
    pub fn display_shape_scope_dependency(&self, scope: usize) -> Option<Span> {
        self.inline_styles.conditional_source(scope)
    }
    pub fn display_shape_seed(&self) -> DisplayShapeSeed {
        DisplayShapeSeed {
            document: self.id,
            revision: self.revision,
            phase: 0,
            at: 0,
            annotations: crate::annotations::ShapeSeed::new(),
            styles: Default::default(),
            steps: 0,
        }
    }
    /// Subscribe after obtaining the current-metadata seed. Complete that seed
    /// before the next append; its revision check prevents mixing both views.
    /// This retains no historical effects and does not inspect source bytes.
    pub fn subscribe_display_shape_effects(&mut self) {
        if self.shape.enabled() {
            return;
        }
        let clock = Arc::new(ShapeClock::default());
        for log in [
            &mut self.shape,
            &mut self.annotations.shape,
            &mut self.display.shape,
            &mut self.inline_styles.shape,
            &mut self.link_order.shape,
            &mut self.references.shape,
            &mut self.footnotes.shape,
        ] {
            log.enable(clock.clone());
        }
    }
}
impl DisplayShapeSeed {
    pub fn work(&self) -> DisplayShapeSeedWork {
        let mut work = self.annotations.work;
        work.steps += self.steps;
        work
    }
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.annotations.capacity_bytes()
    }
    /// Cancel without synchronously dropping a checkpoint's gathered map.
    /// Keep this owner until retirement returns Covered.
    pub fn retire(&mut self, budget: &mut crate::Budget) -> crate::Completion {
        self.phase = 6;
        self.annotations.retire(budget)
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut crate::Budget,
    ) -> Result<Result<Option<DisplayShapeSeedItem>, crate::Completion>, crate::LayoutError> {
        use DisplayShapeSeedItem as Item;
        if document.id != self.document {
            return Err(crate::LayoutError::WrongDocument);
        }
        if document.revision != self.revision {
            return Err(crate::LayoutError::StaleContinuation);
        }
        loop {
            if self.phase == 0 {
                match self.annotations.next(&document.annotations, budget) {
                    Ok(Some(item)) => return Ok(Ok(Some(item))),
                    Err(e) => return Ok(Err(e)),
                    Ok(None) => {
                        self.phase = 1;
                        self.at = 0;
                    }
                }
            }
            if !budget.charge(crate::WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(Err(crate::Completion::BudgetExhausted));
            }
            self.steps += 1;
            match self.phase {
                1 => {
                    if let Some((source, replacement)) = document.display.shape_seed_entry(self.at)
                    {
                        self.at += 1;
                        return Ok(Ok(Some(Item::Replacement {
                            source,
                            replacement,
                        })));
                    }
                    self.phase = 2;
                    self.at = 0;
                }
                2 => {
                    if let Some(&(source, columns)) = document.display.space_mappings().get(self.at)
                    {
                        self.at += 1;
                        return Ok(Ok(Some(Item::Replacement {
                            source,
                            replacement: DisplayShapeReplacement::Spaces { columns },
                        })));
                    }
                    self.phase = 3;
                    self.at = 0;
                }
                3 => match self.styles.next(&document.inline_styles) {
                    Some(Some(source)) => {
                        return Ok(Ok(Some(Item::Candidate {
                            source,
                            conditional_scope: self
                                .styles
                                .conditional_source(&document.inline_styles),
                        })));
                    }
                    Some(None) => {}
                    None => {
                        self.phase = 4;
                        self.at = 0;
                    }
                },
                4 => {
                    if let Some(&info) = document.link_info.get(self.at) {
                        let id = LinkId(self.at);
                        self.at += 1;
                        return Ok(Ok(Some(Item::Link {
                            id,
                            info,
                            active: document.link_order.active(id),
                        })));
                    }
                    self.phase = 5;
                    self.at = 0;
                }
                5 => {
                    if let Some(reference) = document.footnote_references().get(self.at) {
                        let id = FootnoteReferenceId(self.at);
                        self.at += 1;
                        if document.footnote_reference_state(id).is_some() {
                            return Ok(Ok(Some(Item::FootnoteReference {
                                id,
                                source: reference.source,
                            })));
                        }
                    } else {
                        self.phase = 6;
                    }
                }
                _ => return Ok(Ok(None)),
            }
        }
    }
}
