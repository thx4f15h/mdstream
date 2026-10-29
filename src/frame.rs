use crate::folding::{FoldSegment, FoldState};
use crate::{
    Budget, Completion, Document, DocumentId, HeightIndex, LayoutError, LinkId, LinkInfo,
    RenderRun, Span, TileId, WorkLimit,
};
use crate::{FoldPlaceholder, FoldView};

/// Callbacks must themselves be bounded. A run never exceeds the remaining byte
/// allowance; each run/object and each tile is charged before delivery. Expensive
/// shaping/decoding belongs in a resumable task outside these callbacks.
pub trait LayoutSink {
    fn begin_tile(&mut self, tile: TileId);
    fn text(&mut self, run: RenderRun<'_>);
    fn object(&mut self, id: LinkId, object: &LinkInfo);
    fn end_tile(&mut self, tile: TileId) -> u64;
}

/// A folded pass requires an explicit drawable placeholder callback. Account
/// its rows in end_tile, just like text/objects, and retain its source mapping.
pub trait FoldLayoutSink: LayoutSink {
    fn placeholder(&mut self, placeholder: FoldPlaceholder);
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
    active: bool,
    rows: u128,
    wanted: u128,
    approximate: bool,
    recovery_until: usize,
    folded: bool,
    fold_segment: Option<FoldSegment>,
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
            active: false,
            rows: 0,
            wanted: u128::from(rows),
            approximate: start.0 != 0 && !restored,
            recovery_until: start.0,
            folded: false,
            fold_segment: None,
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
            |sink, placeholder| sink.placeholder(placeholder),
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
        placeholder: fn(&mut T, FoldPlaceholder),
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
        if self.rows >= self.wanted {
            return Ok(Completion::Covered);
        }
        if self.tile >= document.tile_count() {
            return Ok(Completion::EndOfDocument);
        }
        loop {
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
                    self.objects_ready = false;
                    continue;
                }
                // Both text and objects participate. Whole hidden interiors are
                // removed by one range operation, independent of their byte count.
                let mut next = document.next_drawable(span.start);
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
                    placeholder(
                        sink,
                        FoldPlaceholder {
                            candidate,
                            owner: TileId(self.tile),
                        },
                    );
                }
                self.cursor = candidate.hidden.end.min(span.end);
                self.objects_ready = false;
                continue;
            }
            if !self.objects_ready {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    recovery,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                self.object = document
                    .links_in(Span::new(self.cursor, document.source().len()))
                    .next()
                    .map_or(document.links().len(), |(id, _)| id.0);
                self.objects_ready = true;
            }
            if let Some(info) = document.link_info(LinkId(self.object))
                && info.source.start <= self.cursor
                && info.source.start < span.end
            {
                if !budget.charge(WorkLimit {
                    objects: 1,
                    recovery,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                sink.object(LinkId(self.object), info);
                self.object += 1;
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
            let run = document
                .runs_in(Span::new(self.cursor, end))
                .unwrap()
                .next()
                .unwrap();
            let consumed = run.source.len();
            let charged = budget.charge(WorkLimit {
                queries: 1,
                bytes: consumed,
                objects: 1,
                recovery,
                ..Default::default()
            });
            debug_assert!(charged);
            self.cursor = run.source.end;
            sink.text(run);
        }
    }
}
