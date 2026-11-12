use std::collections::HashMap;
use std::fmt;

use mdtui3::{
    Budget, Completion, Document, DocumentId, HeightIndex, LayoutError, LayoutPass, Span, TileId,
    WorkLimit,
};

use crate::render::checkpoint::{RenderCheckpoint, RenderCheckpointStore};
use crate::render::context::WindowContexts;
use crate::render::display::{DisplayScanner, ScanStatus};
use crate::render::plan::{BlockPlan, PlanError, TileSegmentKind};
use crate::render::sink::{LayoutSinkStats, SinkError, TileLayoutSink};
use crate::render::table_widths::{TableWidthCache, TableWidthError, TableWidthSnapshot};
use crate::screen::Screen;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstFrameConfig {
    pub top_tile: TileId,
    pub tile_limit: usize,
    pub width: u16,
    pub row_limit: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstFrameStatus {
    PendingScan,
    BudgetExhausted,
    Covered,
    EndOfDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstFrameStep {
    pub status: FirstFrameStatus,
    pub rows: u64,
    pub processed_tiles: usize,
    pub layout_attempts: usize,
    pub direct_tiles: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstFrameError {
    InvalidConfig,
    Stale,
    InvalidTop,
    Plan(PlanError),
    Sink(SinkError),
    Layout(LayoutError),
    MissingSink,
    InvalidReplay,
    TableWidthsLocked,
    TableWidth(TableWidthError),
    TileRowsMismatch {
        tile: TileId,
        expected: u64,
        actual: u64,
    },
    UnexpectedCompletion(Completion),
}

impl fmt::Display for FirstFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid first-frame configuration"),
            Self::Stale => formatter.write_str("first-frame progress is stale"),
            Self::InvalidTop => formatter.write_str("top tile is outside the document"),
            Self::Plan(error) => error.fmt(formatter),
            Self::Sink(error) => error.fmt(formatter),
            Self::Layout(error) => error.fmt(formatter),
            Self::MissingSink => formatter.write_str("layout sink is not ready"),
            Self::InvalidReplay => formatter.write_str("invalid replay range"),
            Self::TableWidth(error) => error.fmt(formatter),
            Self::TableWidthsLocked => {
                formatter.write_str("table widths must be set before layout starts")
            }
            Self::TileRowsMismatch {
                tile,
                expected,
                actual,
            } => write!(
                formatter,
                "tile {} height mismatch: expected {expected}, got {actual}",
                tile.0
            ),
            Self::UnexpectedCompletion(completion) => {
                write!(formatter, "unexpected layout completion: {completion:?}")
            }
        }
    }
}

impl std::error::Error for FirstFrameError {}

impl From<PlanError> for FirstFrameError {
    fn from(error: PlanError) -> Self {
        Self::Plan(error)
    }
}

impl From<SinkError> for FirstFrameError {
    fn from(error: SinkError) -> Self {
        Self::Sink(error)
    }
}

impl From<LayoutError> for FirstFrameError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<TableWidthError> for FirstFrameError {
    fn from(error: TableWidthError) -> Self {
        Self::TableWidth(error)
    }
}

pub struct FirstFrame {
    width_cache: TableWidthCache,
    code_prefix_metadata_reads: usize,
    code_prefix_resolution: mdtui3::LinkResolutionWork,
    document: DocumentId,
    revision: u64,
    config: FirstFrameConfig,
    plan: BlockPlan,
    plan_ready: bool,
    promoted_code: Option<mdtui3::BlockId>,
    skipped_code_prefix: Option<std::ops::Range<usize>>,
    contexts: WindowContexts,
    scanner: DisplayScanner,
    scan_complete: bool,
    sink: Option<TileLayoutSink>,
    pass: Option<LayoutPass>,
    heights: HeightIndex,
    checkpoints: RenderCheckpointStore,
    cursor: usize,
    rows: u64,
    layout_attempts: usize,
    direct_tiles: usize,
    processed_tiles: usize,
    replay: Option<ReplayRequest>,
    replay_pass: Option<LayoutPass>,
    replay_used: bool,
    table_widths: HashMap<mdtui3::TableId, TableWidthSnapshot>,
    suppressed_heading_tiles: Vec<std::ops::Range<usize>>,
}

#[derive(Debug, Clone, Copy)]
struct ReplayRequest {
    from: TileId,
    target: TileId,
    checkpoint: RenderCheckpoint,
    max_bytes: usize,
}

impl FirstFrame {
    pub fn new(document: &Document, config: FirstFrameConfig) -> Result<Self, FirstFrameError> {
        if config.tile_limit == 0 || config.width == 0 || config.row_limit == 0 {
            return Err(FirstFrameError::InvalidConfig);
        }
        if config.top_tile.0 > document.tile_count() {
            return Err(FirstFrameError::InvalidTop);
        }
        let original_start = config.top_tile.0;
        // Confirmed fence markers and info are indexed hidden spans. Jump over
        // a long hidden prefix using metadata, while keeping its bounded label
        // in the first body tile. Never read the hidden line to find the body.
        let mut code_prefix_metadata_reads = 0;
        let mut code_prefix_resolution = mdtui3::LinkResolutionWork::default();
        let promoted = document
            .tile(config.top_tile)
            .and_then(|span| document.block_at(span.start))
            .and_then(|id| document.block(id).map(|block| (id, block)))
            .filter(|(_, block)| matches!(block.kind, mdtui3::BlockKind::Code { .. }))
            .and_then(|(id, block)| {
                // A confirmed code span has unconditional literal annotations.
                // Limit the query before reading, including an empty body: a
                // following paragraph must never be evaluated by this probe.
                let mut runs = document.runs_in(block.source)?;
                let drawable = runs
                    .by_ref()
                    .take(3)
                    .inspect(|_| code_prefix_metadata_reads += 1)
                    .find(|run| !run.style.contains(mdtui3::Style::HIDDEN))
                    .map(|run| run.source.start);
                code_prefix_resolution = runs.link_resolution_work();
                let at = drawable.unwrap_or_else(|| block.source.end.saturating_sub(1));
                let tile = document.tile_at(at)?;
                (tile.0 >= original_start.saturating_add(config.tile_limit)).then_some((id, tile.0))
            });
        let start = promoted.map_or(original_start, |(_, tile)| tile);
        let promoted_code = promoted.map(|(id, _)| id);
        let skipped_code_prefix = (start > original_start).then_some(original_start..start);
        let end = start
            .saturating_add(config.tile_limit)
            .min(document.tile_count());
        let source = if start == end {
            Span::new(document.source().len(), document.source().len())
        } else {
            let first = document
                .tile(TileId(start))
                .ok_or(FirstFrameError::InvalidTop)?;
            let last = document
                .tile(TileId(end - 1))
                .ok_or(FirstFrameError::InvalidTop)?;
            Span::new(first.start, last.end)
        };
        let contexts = WindowContexts::new(document, source);
        let plan_ready = contexts.is_empty();
        let plan = if plan_ready {
            crate::render::plan::plan_with_contexts(
                document,
                source,
                &contexts.entries,
                promoted_code,
            )?
        } else {
            crate::render::plan::empty_plan(document, source, start, end)
        };
        let scanner = DisplayScanner::new(document, source).ok_or(FirstFrameError::InvalidTop)?;
        let mut heights = HeightIndex::default();
        heights.rebuild(document)?;
        let mut suppressed_heading_tiles = suppress_nonwrapping(document, &plan, &mut heights)?;
        if let Some(range) = &skipped_code_prefix {
            heights.suppress(range.clone())?;
            suppressed_heading_tiles.push(range.clone());
        }
        Ok(Self {
            width_cache: TableWidthCache::new(document),
            code_prefix_metadata_reads,
            code_prefix_resolution,
            document: document.id(),
            revision: document.revision(),
            config,
            plan,
            plan_ready,
            promoted_code,
            skipped_code_prefix,
            contexts,
            scanner,
            scan_complete: false,
            sink: None,
            pass: None,
            heights,
            checkpoints: RenderCheckpointStore::new(document),
            cursor: 0,
            rows: 0,
            layout_attempts: 0,
            direct_tiles: 0,
            processed_tiles: 0,
            replay: None,
            replay_pass: None,
            replay_used: false,
            table_widths: HashMap::new(),
            suppressed_heading_tiles,
        })
    }

    pub fn code_prefix_work(&self) -> (usize, mdtui3::LinkResolutionWork) {
        (self.code_prefix_metadata_reads, self.code_prefix_resolution)
    }

    pub fn shared_links(&self) -> crate::render::objects::SharedLinks {
        self.scanner.shared_links()
    }
    pub fn set_links(&mut self, links: crate::render::objects::SharedLinks) {
        self.width_cache.set_links(links.clone());
        self.scanner.set_links(links);
    }

    #[must_use]
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    #[must_use]
    pub fn plan(&self) -> &BlockPlan {
        &self.plan
    }

    #[must_use]
    pub fn scanner(&self) -> &DisplayScanner {
        &self.scanner
    }

    #[must_use]
    pub const fn scan_complete(&self) -> bool {
        self.scan_complete
    }

    #[must_use]
    pub fn sink(&self) -> Option<&TileLayoutSink> {
        self.sink.as_ref()
    }

    #[must_use]
    pub fn lines(&self) -> &[crate::render::sink::SinkLine] {
        self.sink.as_ref().map_or(&[], TileLayoutSink::lines)
    }

    pub fn write_to_screen(&self, screen: &mut Screen, start_row: u16) -> Result<u16, SinkError> {
        self.sink
            .as_ref()
            .map_or(Ok(0), |sink| sink.write_to_screen(screen, start_row))
    }

    #[must_use]
    pub fn sink_stats(&self) -> Option<LayoutSinkStats> {
        self.sink.as_ref().map(|sink| {
            let mut stats = sink.stats();
            stats.objects = self.scanner.shared_links().stats();
            stats.object_index = self.scanner.object_index_work();
            stats
        })
    }

    #[must_use]
    pub const fn rows(&self) -> u64 {
        self.rows
    }

    #[must_use]
    pub const fn processed_tiles(&self) -> usize {
        self.processed_tiles
    }

    #[must_use]
    pub const fn layout_attempts(&self) -> usize {
        self.layout_attempts
    }

    #[must_use]
    pub const fn direct_tiles(&self) -> usize {
        self.direct_tiles
    }

    #[must_use]
    pub const fn replay_used(&self) -> bool {
        self.replay_used
    }

    #[must_use]
    pub fn heights(&self) -> &HeightIndex {
        &self.heights
    }

    #[must_use]
    pub fn checkpoints(&self) -> &RenderCheckpointStore {
        &self.checkpoints
    }

    #[must_use]
    pub fn into_layout_parts(self) -> (HeightIndex, RenderCheckpointStore) {
        (self.heights, self.checkpoints)
    }

    pub fn suppressed_heading_tiles(&self) -> &[std::ops::Range<usize>] {
        &self.suppressed_heading_tiles
    }

    pub fn begin_replay(
        &mut self,
        from: TileId,
        target: TileId,
        checkpoint: RenderCheckpoint,
        max_bytes: usize,
    ) -> Result<(), FirstFrameError> {
        if from.0 < self.plan.tile_start.0
            || target < from
            || target.0 >= self.plan.tile_end.0
            || max_bytes == 0
            || self
                .plan
                .segments
                .get(
                    from.0.saturating_sub(self.plan.tile_start.0)
                        ..target
                            .0
                            .saturating_sub(self.plan.tile_start.0)
                            .saturating_add(1),
                )
                .is_none_or(|segments| {
                    segments
                        .iter()
                        .any(|segment| segment.kind == TileSegmentKind::TuiOwned)
                })
        {
            return Err(FirstFrameError::InvalidReplay);
        }
        self.replay = Some(ReplayRequest {
            from,
            target,
            checkpoint,
            max_bytes,
        });
        self.replay_pass = None;
        self.replay_used = false;
        if let Some(sink) = &mut self.sink {
            sink.restore_checkpoint(checkpoint);
        }
        Ok(())
    }

    pub fn set_table_widths(
        &mut self,
        table_widths: HashMap<mdtui3::TableId, TableWidthSnapshot>,
    ) -> Result<(), FirstFrameError> {
        if self.sink.is_some() || self.processed_tiles > 0 {
            return Err(FirstFrameError::TableWidthsLocked);
        }
        self.table_widths = table_widths;
        Ok(())
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<FirstFrameStep, FirstFrameError> {
        if !self.matches(document) {
            return Err(FirstFrameError::Stale);
        }
        if let Some(status) = self.ensure_ready(document, budget)? {
            return Ok(self.step(status));
        }
        if self.replay.is_some()
            && !self.replay_used
            && let Some(status) = self.advance_replay(document, budget)?
        {
            return Ok(self.step(status));
        }
        loop {
            if self.rows >= self.config.row_limit {
                return Ok(self.step(FirstFrameStatus::Covered));
            }
            let Some(segment) = self.plan.segments.get(self.cursor) else {
                let status = if self.plan.tile_end.0 >= document.tile_count() {
                    FirstFrameStatus::EndOfDocument
                } else {
                    FirstFrameStatus::Covered
                };
                return Ok(self.step(status));
            };
            let tile = segment.tile;
            let sink = self.sink.as_ref().ok_or(FirstFrameError::MissingSink)?;
            let expected = sink.tile_rows(tile).ok_or(FirstFrameError::MissingSink)?;

            if segment.kind == TileSegmentKind::Drawable && expected > 0 {
                if self.pass.is_none() {
                    self.layout_attempts = self.layout_attempts.saturating_add(1);
                    let restored = tile.0 == 0 || self.checkpoints.exact(tile).is_some();
                    self.pass = Some(LayoutPass::new(
                        document,
                        &self.heights,
                        tile,
                        expected,
                        restored,
                    )?);
                }
                let completion = {
                    let pass = self.pass.as_mut().ok_or(FirstFrameError::MissingSink)?;
                    let sink = self.sink.as_mut().ok_or(FirstFrameError::MissingSink)?;
                    self.scanner.shared_links().with_resolver(|resolver| {
                        pass.swap_link_resolver(resolver)?;
                        let result = pass.advance(document, &mut self.heights, budget, sink);
                        pass.swap_link_resolver(resolver)?;
                        result
                    })?
                };
                match completion {
                    Completion::Covered
                        if self
                            .pass
                            .as_ref()
                            .is_some_and(|pass| pass.next_tile() == TileId(tile.0 + 1)) =>
                    {
                        self.pass = None;
                    }
                    Completion::BudgetExhausted => {
                        return Ok(self.step(FirstFrameStatus::BudgetExhausted));
                    }
                    completion => {
                        return Err(FirstFrameError::UnexpectedCompletion(completion));
                    }
                }
                let actual = self.heights.height(tile).unwrap_or(0);
                if actual != expected {
                    return Err(FirstFrameError::TileRowsMismatch {
                        tile,
                        expected,
                        actual,
                    });
                }
            } else {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    tiles: 1,
                    objects: 1,
                    ..WorkLimit::default()
                }) {
                    return Ok(self.step(FirstFrameStatus::BudgetExhausted));
                }
                let sink = self.sink.as_mut().ok_or(FirstFrameError::MissingSink)?;
                let actual = sink.render_tui_owned_tile(tile);
                if actual != expected {
                    return Err(FirstFrameError::TileRowsMismatch {
                        tile,
                        expected,
                        actual,
                    });
                }
                let ticket = self
                    .heights
                    .ticket(tile)
                    .ok_or(FirstFrameError::InvalidTop)?;
                self.heights.measure(ticket, actual)?;
                self.direct_tiles = self.direct_tiles.saturating_add(1);
            }

            if self
                .sink
                .as_ref()
                .is_some_and(|sink| sink.stats().edge_placeholders > 0)
                && let Some(ticket) = self.heights.ticket(tile)
            {
                self.heights.measure_local(ticket, expected)?;
            }
            let checkpoint = self
                .sink
                .as_ref()
                .ok_or(FirstFrameError::MissingSink)?
                .checkpoint();
            self.checkpoints
                .checkpoint_after_tile(tile, checkpoint, document)?;
            self.rows = self.rows.saturating_add(expected);
            self.processed_tiles = self.processed_tiles.saturating_add(1);
            self.cursor += 1;
        }
    }

    fn ensure_ready(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Option<FirstFrameStatus>, FirstFrameError> {
        if !self.plan_ready {
            if !self.contexts.advance(document, budget) {
                return Ok(Some(FirstFrameStatus::BudgetExhausted));
            }
            self.plan = crate::render::plan::plan_with_contexts(
                document,
                self.scanner.requested(),
                &self.contexts.entries,
                self.promoted_code,
            )?;
            self.suppressed_heading_tiles =
                suppress_nonwrapping(document, &self.plan, &mut self.heights)?;
            if let Some(range) = &self.skipped_code_prefix {
                self.suppressed_heading_tiles.push(range.clone());
            }
            self.plan_ready = true;
        }
        if !self.plan.advance_info(document, budget) {
            return Ok(Some(FirstFrameStatus::BudgetExhausted));
        }
        if !self.scan_complete {
            match self.scanner.advance(document, budget)? {
                ScanStatus::BudgetExhausted => return Ok(Some(FirstFrameStatus::PendingScan)),
                ScanStatus::Complete => self.scan_complete = true,
            }
        }
        if self.sink.is_none() {
            for block in &self.plan.blocks {
                if let mdtui3::BlockKind::Table { id } = block.kind
                    && !self.table_widths.contains_key(&id)
                {
                    let Some(snapshot) = self.width_cache.snapshot_budgeted(
                        document,
                        id,
                        usize::from(self.config.width),
                        budget,
                    )?
                    else {
                        return Ok(Some(FirstFrameStatus::BudgetExhausted));
                    };
                    self.table_widths.insert(id, snapshot);
                }
            }
            // Dynamic selection is now prepared. Classify from that window,
            // rather than evaluating unbudgeted next_drawable predicates.
            let mut drawable = vec![None; self.plan.segments.len()];
            for segment in self.scanner.segments() {
                if let Some(index) = segment
                    .tile
                    .0
                    .checked_sub(self.plan.tile_start.0)
                    .filter(|&index| index < drawable.len())
                {
                    drawable[index].get_or_insert(segment.source.start);
                }
            }
            let mut events = vec![false; self.plan.segments.len()];
            for event in &self.plan.events {
                if let Some(tile) = document.tile_at(event.source.start)
                    && let Some(index) = tile
                        .0
                        .checked_sub(self.plan.tile_start.0)
                        .filter(|&index| index < events.len())
                {
                    events[index] = true;
                }
            }
            for (index, segment) in self.plan.segments.iter_mut().enumerate() {
                segment.drawable_at = drawable[index];
                segment.kind = if drawable[index].is_some() {
                    TileSegmentKind::Drawable
                } else if events[index] {
                    TileSegmentKind::TuiOwned
                } else {
                    TileSegmentKind::ZeroRow
                };
            }
            if !budget.charge(WorkLimit {
                objects: 1,
                ..WorkLimit::default()
            }) {
                return Ok(Some(FirstFrameStatus::BudgetExhausted));
            }
            self.sink = Some(TileLayoutSink::new_with_contexts(
                document,
                &self.plan,
                &self.scanner,
                self.config.width,
                &self.table_widths,
                &self.contexts.entries,
                self.contexts.reads,
            )?);
            if let Some(replay) = self.replay
                && !self.replay_used
            {
                self.sink
                    .as_mut()
                    .ok_or(FirstFrameError::MissingSink)?
                    .restore_checkpoint(replay.checkpoint);
            }
        }
        Ok(None)
    }

    fn advance_replay(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Option<FirstFrameStatus>, FirstFrameError> {
        let replay = self.replay.ok_or(FirstFrameError::InvalidReplay)?;
        if self
            .sink
            .as_ref()
            .and_then(|sink| sink.tile_rows(replay.target))
            .unwrap_or(0)
            == 0
        {
            self.replay = None;
            return Ok(None);
        }
        if self.replay_pass.is_none() {
            let Some(pass) = LayoutPass::from_checkpoint(
                document,
                &self.heights,
                replay.from,
                replay.target,
                1,
                replay.max_bytes,
            )?
            else {
                return Err(FirstFrameError::InvalidReplay);
            };
            self.replay_pass = Some(pass);
        }
        let completion = {
            let pass = self
                .replay_pass
                .as_mut()
                .ok_or(FirstFrameError::InvalidReplay)?;
            let sink = self.sink.as_mut().ok_or(FirstFrameError::MissingSink)?;
            self.scanner.shared_links().with_resolver(|resolver| {
                pass.swap_link_resolver(resolver)?;
                let result = pass.advance(document, &mut self.heights, budget, sink);
                pass.swap_link_resolver(resolver)?;
                result
            })?
        };
        match completion {
            Completion::BudgetExhausted => Ok(Some(FirstFrameStatus::BudgetExhausted)),
            Completion::Covered | Completion::EndOfDocument => {
                let next = self
                    .replay_pass
                    .as_ref()
                    .ok_or(FirstFrameError::InvalidReplay)?
                    .next_tile();
                let measured = self
                    .replay_pass
                    .as_ref()
                    .ok_or(FirstFrameError::InvalidReplay)?
                    .measured_rows();
                self.replay_pass = None;
                self.replay_used = true;
                self.rows = u64::try_from(measured).unwrap_or(u64::MAX);
                self.cursor = next
                    .0
                    .saturating_sub(self.plan.tile_start.0)
                    .min(self.plan.segments.len());
                self.processed_tiles = self.processed_tiles.max(
                    replay
                        .target
                        .0
                        .saturating_sub(replay.from.0)
                        .saturating_add(1),
                );
                let checkpoint = self
                    .sink
                    .as_ref()
                    .ok_or(FirstFrameError::MissingSink)?
                    .checkpoint();
                self.checkpoints
                    .checkpoint_after_tile(replay.target, checkpoint, document)?;
                Ok(None)
            }
        }
    }

    fn step(&self, status: FirstFrameStatus) -> FirstFrameStep {
        FirstFrameStep {
            status,
            rows: self.rows,
            processed_tiles: self.processed_tiles,
            layout_attempts: self.layout_attempts,
            direct_tiles: self.direct_tiles,
        }
    }
}

fn suppress_nonwrapping(
    document: &Document,
    plan: &BlockPlan,
    heights: &mut HeightIndex,
) -> Result<Vec<std::ops::Range<usize>>, FirstFrameError> {
    let mut ranges = Vec::new();
    for block in &plan.blocks {
        let mut sources = Vec::new();
        match block.kind {
            mdtui3::BlockKind::Heading { .. } => sources.push(block.source),
            mdtui3::BlockKind::Table { id } => {
                if let Some(table) = document.table(id) {
                    let window = crate::render::table::table_row_window(table, plan.visible);
                    sources.extend(table.rows()[window].iter().map(|row| row.source));
                }
            }
            _ => {}
        }
        for source in sources {
            if source.len() <= 4096 {
                continue;
            }
            let first = document
                .tile_at(source.start)
                .ok_or(FirstFrameError::InvalidTop)?;
            let last = document
                .tile_at(source.end - 1)
                .ok_or(FirstFrameError::InvalidTop)?;
            let last_span = document.tile(last).ok_or(FirstFrameError::InvalidTop)?;
            let end = last.0 + usize::from(last_span.end <= source.end);
            let range = first.0 + 1..end;
            if !range.is_empty() {
                heights.suppress(range.clone())?;
                ranges.push(range);
            }
        }
    }
    Ok(ranges)
}
