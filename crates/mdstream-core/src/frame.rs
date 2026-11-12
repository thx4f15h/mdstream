use crate::folding::{FoldSegment, FoldState};
use crate::{
    BlockId, Budget, Completion, Document, DocumentId, HeightIndex, LayoutError, LinkId, LinkInfo,
    RenderRun, Span, Style, TileId, WorkLimit,
};
use crate::{FoldPlaceholder, FoldView};

/// Callbacks must themselves be bounded. A run never exceeds the remaining byte
/// allowance; each run/object and each tile is charged before delivery. Expensive
/// shaping/decoding belongs in a resumable task outside these callbacks.
pub trait LayoutSink {
    fn begin_tile(&mut self, tile: TileId);
    fn text(&mut self, run: RenderRun<'_>);
    fn object(&mut self, id: LinkId, object: &LinkInfo);
    /// Selected candidates only. Unresolved keeps literal source semantics;
    /// suppressed alternatives never reach this callback. The default preserves
    /// existing sinks, while hosts may distinguish activation without requerying.
    fn object_with_disposition(
        &mut self,
        id: LinkId,
        object: &LinkInfo,
        disposition: crate::LinkDisposition,
    ) {
        let _ = disposition;
        self.object(id, object);
    }
    fn end_tile(&mut self, tile: TileId) -> u64;
}

/// A folded pass requires an explicit drawable placeholder callback. Account
/// its rows in end_tile, just like text/objects, and retain its source mapping.
pub trait FoldLayoutSink: LayoutSink {
    fn placeholder(&mut self, placeholder: FoldPlaceholder);
    /// State-only reset after skipping a registered paragraph's hidden suffix,
    /// before any following text. Also delivered when starting inside the tail
    /// without its placeholder. Reset wrapping/paragraph-local state here; do
    /// not add rows (the placeholder's owner accounts for its terminating line).
    /// One object/recovery step, resumable independently of placeholder delivery.
    fn paragraph_end(&mut self, _block: BlockId) {}
}

enum FoldEvent {
    Placeholder(FoldPlaceholder),
    ParagraphEnd(BlockId),
}

/// A resumable local layout pass. Source position, partial tile and sink state
/// survive frame boundaries, so a tiny positive budget does not rescan a prefix.
/// The caller requests a local number of rows, never an unconditional EOF sweep.
/// Remote starts have approximate incoming wrapping state unless the caller has
/// restored a nearby checkpoint. Absolute prefix heights remain estimates.
pub struct LayoutPass {
    document: DocumentId,
    revision: u64,
    view: (DocumentId, u64),
    tile: usize,
    cursor: usize,
    object: usize,
    objects_ready: bool,
    object_window: Option<crate::LinkWindow>,
    last_object: Option<(usize, usize)>,
    resolver: crate::LinkResolver,
    object_activity: Option<crate::LinkActivity>,
    pending_run: Option<PreparedLayout>,
    active: bool,
    rows: u128,
    wanted: u128,
    approximate: bool,
    recovery_until: usize,
    folded: bool,
    fold_segment: Option<FoldSegment>,
    paragraph_end: Option<BlockId>,
    ended_paragraph: Option<BlockId>,
}

struct PreparedLayout {
    source: Span,
    block: Option<BlockId>,
    style: Style,
    emphasis_depth: usize,
    strong_depth: usize,
    task: crate::link_semantics::MarkTask,
}
impl PreparedLayout {
    fn new(run: RenderRun<'_>, mark: crate::annotations::Mark) -> Self {
        Self {
            source: run.source,
            block: run.block,
            style: run.style,
            emphasis_depth: run.emphasis_depth,
            strong_depth: run.strong_depth,
            task: crate::link_semantics::MarkTask::new(mark),
        }
    }
    fn emit<'a>(&mut self, document: &'a Document, end: usize) -> RenderRun<'a> {
        let source = Span::new(self.source.start, end);
        self.source.start = end;
        let (hidden, link) = self.task.result.unwrap();
        RenderRun {
            source,
            text: &document.source()[source.range()],
            block: self.block,
            style: if hidden {
                self.style.union(Style::HIDDEN)
            } else {
                self.style
            },
            emphasis_depth: self.emphasis_depth,
            strong_depth: self.strong_depth,
            link,
        }
    }
}

impl LayoutPass {
    pub fn new(
        document: &Document,
        heights: &HeightIndex,
        start: TileId,
        rows: u64,
        restored: bool,
    ) -> Result<Self, LayoutError> {
        if start.0 > document.tile_count() || heights.len() != document.tile_count() {
            return Err(LayoutError::InvalidRange);
        }
        heights.check_document(document)?;
        let cursor = document
            .tile(start)
            .map_or(document.source().len(), |s| s.start);
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: heights.version(),
            tile: start.0,
            cursor,
            object: 0,
            objects_ready: false,
            object_window: None,
            last_object: None,
            resolver: document.link_resolver(),
            object_activity: None,
            pending_run: None,
            active: false,
            rows: 0,
            wanted: u128::from(rows),
            approximate: start.0 != 0 && !restored,
            recovery_until: start.0,
            folded: false,
            fold_segment: None,
            paragraph_end: None,
            ended_paragraph: None,
        })
    }

    /// A local pass using this view's visible heights and fold boundaries.
    /// A start inside hidden content skips to its end; only the owner tile emits
    /// the placeholder. Use the view's height index to choose a visible start.
    pub fn folded<S>(
        document: &Document,
        view: &FoldView<S>,
        start: TileId,
        rows: u64,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let mut pass = Self::new(document, view.heights(), start, rows, false)?;
        pass.folded = true;
        Ok(pass)
    }

    pub fn advance_folded<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
        sink: &mut impl FoldLayoutSink,
    ) -> Result<Completion, LayoutError> {
        view.check(document)?;
        self.advance_inner(
            document,
            &mut view.heights,
            budget,
            sink,
            Some(&view.state),
            |sink, event| match event {
                FoldEvent::Placeholder(p) => sink.placeholder(p),
                FoldEvent::ParagraphEnd(block) => sink.paragraph_end(block),
            },
        )
    }

    /// The caller restores `view.checkpoints().previous(input)` into its sink.
    /// Only an exact valid input checkpoint within max_bytes can be used.
    pub fn folded_from_checkpoint<S>(
        document: &Document,
        view: &FoldView<S>,
        input: TileId,
        target: TileId,
        rows: u64,
        max_bytes: usize,
    ) -> Result<Option<Self>, LayoutError> {
        view.check(document)?;
        if view.checkpoints().token(input).is_none() {
            return Ok(None);
        }
        let Some(mut pass) =
            Self::from_checkpoint(document, view.heights(), input, target, rows, max_bytes)?
        else {
            return Ok(None);
        };
        pass.folded = true;
        Ok(Some(pass))
    }

    /// Reuse this pass's revision-bound conditional memo for display and hits.
    /// Move the memo out in O(1) before retiring/replacing this pass.
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        let empty = self.resolver.empty_like();
        std::mem::replace(&mut self.resolver, empty)
    }
    pub fn swap_link_resolver(
        &mut self,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        resolver.check_binding(self.document, self.revision)?;
        std::mem::swap(&mut self.resolver, resolver);
        Ok(())
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.resolver.retire(budget)
    }
    pub fn link_resolver(&mut self) -> &mut crate::LinkResolver {
        &mut self.resolver
    }

    pub fn source_position(&self) -> usize {
        self.cursor
    }
    pub fn next_tile(&self) -> TileId {
        TileId(self.tile)
    }
    pub fn is_approximate(&self) -> bool {
        self.approximate
    }
    pub fn measured_rows(&self) -> u128 {
        self.rows
    }

    /// Replay a nearby checkpoint before the requested tile. The caller restores
    /// the sink state; every replay step consumes the same frame's recovery budget.
    pub fn from_checkpoint(
        document: &Document,
        heights: &HeightIndex,
        input: TileId,
        target: TileId,
        rows: u64,
        max_bytes: usize,
    ) -> Result<Option<Self>, LayoutError> {
        let from = document.tile(input).ok_or(LayoutError::InvalidRange)?;
        let to = document.tile(target).ok_or(LayoutError::InvalidRange)?;
        if input > target {
            return Err(LayoutError::InvalidRange);
        }
        if to.start - from.start > max_bytes {
            return Ok(None);
        }
        let mut pass = Self::new(document, heights, input, rows, true)?;
        pass.recovery_until = target.0;
        Ok(Some(pass))
    }

    pub fn advance(
        &mut self,
        document: &Document,
        heights: &mut HeightIndex,
        budget: &mut Budget,
        sink: &mut impl LayoutSink,
    ) -> Result<Completion, LayoutError> {
        self.advance_inner(document, heights, budget, sink, None, |_, _| unreachable!())
    }

    fn advance_inner<T: LayoutSink>(
        &mut self,
        document: &Document,
        heights: &mut HeightIndex,
        budget: &mut Budget,
        sink: &mut T,
        folds: Option<&FoldState>,
        fold_event: fn(&mut T, FoldEvent),
    ) -> Result<Completion, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.view.0 != heights.version().0 {
            return Err(LayoutError::WrongView);
        }
        if self.folded != folds.is_some() {
            return Err(LayoutError::WrongView);
        }
        if self.revision != document.revision() || self.view != heights.version() {
            return Err(LayoutError::StaleContinuation);
        }
        self.resolver.check(document)?;
        loop {
            if let Some(pending) = &mut self.pending_run {
                let recovery = usize::from(self.tile < self.recovery_until);
                if !pending
                    .task
                    .advance(document, &mut self.resolver.resolution, budget, recovery)
                {
                    return Ok(Completion::BudgetExhausted);
                }
                let mut end = self
                    .cursor
                    .saturating_add(budget.remaining().bytes)
                    .min(pending.source.end);
                while !document.source().is_char_boundary(end) {
                    end -= 1;
                }
                if end == self.cursor
                    || !budget.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        bytes: end - self.cursor,
                        recovery,
                        ..Default::default()
                    })
                {
                    return Ok(Completion::BudgetExhausted);
                }
                let run = pending.emit(document, end);
                self.cursor = end;
                if pending.source.is_empty() {
                    self.pending_run = None;
                }
                sink.text(run);
                continue;
            }
            if let Some(block) = self.paragraph_end {
                // A crossing union can extend the same owner's frontier over
                // several paid steps. End its paragraph once, also when this
                // pass starts remotely in a later extension of that union.
                if self.ended_paragraph == Some(block) {
                    self.paragraph_end = None;
                    continue;
                }
                if !budget.charge(WorkLimit {
                    objects: 1,
                    recovery: usize::from(self.tile < self.recovery_until),
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                fold_event(sink, FoldEvent::ParagraphEnd(block));
                self.paragraph_end = None;
                self.ended_paragraph = Some(block);
            }
            if self.rows >= self.wanted {
                return Ok(Completion::Covered);
            }
            let Some(span) = document.tile(TileId(self.tile)) else {
                return Ok(Completion::EndOfDocument);
            };
            let recovery = usize::from(self.tile < self.recovery_until);
            if self.active && self.cursor == span.end {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    recovery,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                let rows = sink.end_tile(TileId(self.tile));
                let ticket = heights
                    .ticket(TileId(self.tile))
                    .ok_or(LayoutError::InvalidRange)?;
                if self.approximate {
                    heights.measure_local(ticket, rows)?;
                } else {
                    heights.measure(ticket, rows)?;
                }
                if recovery == 0 {
                    self.rows += u128::from(rows);
                }
                self.tile += 1;
                self.active = false;
                continue;
            }
            if let Some(folds) = folds
                && self
                    .fold_segment
                    .is_none_or(|segment| self.cursor >= segment.end())
            {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    recovery,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                self.fold_segment = Some(folds.segment(document, self.cursor));
            }
            if !self.active {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    tiles: 1,
                    recovery,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                if let Some(FoldSegment::Hidden(candidate)) = self.fold_segment
                    && self.cursor > candidate.hidden.start
                    && candidate.hidden.end >= span.end
                {
                    self.tile = document
                        .tile_at(candidate.hidden.end)
                        .map_or(document.tile_count(), |t| t.0);
                    self.cursor = document
                        .tile(TileId(self.tile))
                        .map_or(document.source().len(), |s| s.start);
                    if self.cursor == candidate.hidden.end {
                        self.paragraph_end = folds.and_then(|f| f.paragraph(candidate.id));
                    }
                    self.objects_ready = false;
                    self.object_window = None;
                    continue;
                }
                // Both text and objects participate. Whole hidden interiors are
                // removed by one range operation, independent of their byte count.
                let mut next = document.next_layout_anchor(span.start);
                // Fold placeholders are drawable even when all underlying
                // syntax is hidden (e.g. an empty fenced code block).
                if let Some(segment) = self.fold_segment {
                    let boundary = match segment {
                        FoldSegment::Text { end } => end,
                        FoldSegment::Hidden(c) => c.hidden.start.max(self.cursor),
                    };
                    if boundary < document.source().len() {
                        next = Some(next.unwrap_or(usize::MAX).min(boundary));
                    }
                }
                if next.is_none_or(|at| at >= span.end) {
                    let end = next
                        .and_then(|at| document.tile_at(at))
                        .map_or(document.tile_count(), |tile| tile.0);
                    heights.suppress(self.tile..end)?;
                    self.view = heights.version();
                    self.tile = end;
                    self.cursor = document
                        .tile(TileId(end))
                        .map_or(document.source().len(), |s| s.start);
                    continue;
                }
                sink.begin_tile(TileId(self.tile));
                self.active = true;
            }
            if let Some(FoldSegment::Hidden(candidate)) = self.fold_segment {
                if self.cursor == candidate.hidden.start {
                    if !budget.charge(WorkLimit {
                        objects: 1,
                        recovery,
                        ..Default::default()
                    }) {
                        return Ok(Completion::BudgetExhausted);
                    }
                    fold_event(
                        sink,
                        FoldEvent::Placeholder(FoldPlaceholder {
                            candidate,
                            owner: TileId(self.tile),
                            paragraph: folds.and_then(|f| f.paragraph(candidate.id)),
                        }),
                    );
                }
                self.cursor = candidate.hidden.end.min(span.end);
                if self.cursor == candidate.hidden.end {
                    self.paragraph_end = folds.and_then(|f| f.paragraph(candidate.id));
                }
                self.objects_ready = false;
                self.object_window = None;
                continue;
            }
            if !self.objects_ready {
                let window = self.object_window.get_or_insert_with(|| {
                    document.link_window(Span::new(self.cursor, document.source().len()))
                });
                match window.advance_with_recovery(document, budget, recovery)? {
                    crate::LinkStep::Object(id, _) => self.object = id.0,
                    crate::LinkStep::Complete => self.object = document.links().len(),
                    crate::LinkStep::BudgetExhausted => return Ok(Completion::BudgetExhausted),
                }
                self.objects_ready = true;
            }
            if let Some(info) = document.link_info(LinkId(self.object))
                && info.source.start <= self.cursor
                && info.source.start < span.end
            {
                let key = (info.source.start, self.object);
                if self.last_object.is_some_and(|last| key <= last) {
                    if !budget.charge(WorkLimit {
                        objects: 1,
                        recovery,
                        ..Default::default()
                    }) {
                        return Ok(Completion::BudgetExhausted);
                    }
                    self.objects_ready = false;
                    self.object_activity = None;
                    continue;
                }
                let disposition = if document
                    .link_order
                    .gate(LinkId(self.object))
                    .unconditional()
                {
                    if crate::link_semantics::available(document, LinkId(self.object)) {
                        crate::LinkDisposition::Active
                    } else {
                        crate::LinkDisposition::Unresolved
                    }
                } else {
                    let activity = self.object_activity.get_or_insert_with(|| {
                        document.link_activity(LinkId(self.object)).unwrap()
                    });
                    let Some(disposition) = activity.advance_in(
                        document,
                        &mut self.resolver.resolution,
                        budget,
                        recovery,
                    )?
                    else {
                        return Ok(Completion::BudgetExhausted);
                    };
                    disposition
                };
                if !budget.charge(WorkLimit {
                    objects: 1,
                    recovery,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                if disposition != crate::LinkDisposition::Suppressed {
                    sink.object_with_disposition(LinkId(self.object), info, disposition);
                }
                self.last_object = Some(key);
                self.objects_ready = false;
                self.object_activity = None;
                continue;
            }
            let remaining = budget.remaining();
            if remaining.objects == 0
                || remaining.bytes == 0
                || remaining.queries == 0
                || remaining.recovery < recovery
            {
                return Ok(Completion::BudgetExhausted);
            }
            let mut end = self.cursor.saturating_add(remaining.bytes).min(span.end);
            if let Some(segment) = self.fold_segment {
                end = end.min(segment.end());
            }
            if let Some(info) = document.link_info(LinkId(self.object))
                && info.source.start > self.cursor
            {
                end = end.min(info.source.start);
            }
            while !document.source().is_char_boundary(end) {
                end -= 1;
            }
            if end == self.cursor {
                return Ok(Completion::BudgetExhausted);
            }
            let (run, mark) = {
                let mut runs = document.runs_in(Span::new(self.cursor, end)).unwrap();
                runs.raw = true;
                let run = runs.next().unwrap();
                let mark = runs.last_mark;
                (run, mark)
            };
            let consumed = run.source.len();
            let charged = budget.charge(WorkLimit {
                queries: 1,
                bytes: consumed,
                objects: 1,
                recovery,
                ..Default::default()
            });
            debug_assert!(charged);
            if crate::link_semantics::Resolution::needs(document, mark) {
                self.pending_run = Some(PreparedLayout::new(run, mark));
            } else {
                let mut run = run;
                run.style = run
                    .style
                    .union(mark.effective_style(&document.references.entries, &document.links));
                self.cursor = run.source.end;
                sink.text(run);
            }
        }
    }
}
