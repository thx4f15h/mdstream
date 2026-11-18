use std::ops::Range;

use mdtui3::{
    ArenaStats, BlockId, CheckpointIndex, CheckpointToken, Document, LayoutError, TileId, Update,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckpointPosition {
    #[default]
    LineStart,
    LineMid,
    BlockInner,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckpointBlockKind {
    #[default]
    None,
    Paragraph,
    Heading,
    Quote,
    ListItem,
    Code,
    Table,
    ThematicBreak,
    Math,
    Blank,
    Definition,
    FootnoteDefinition,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ListMarkerState {
    #[default]
    None,
    Bullet,
    Ordered,
    TaskUnchecked,
    TaskChecked,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CodeRenderState {
    #[default]
    None,
    InfoLabel,
    Body,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableRenderState {
    #[default]
    None,
    Header,
    Body,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PendingBlockEvents {
    pub spacing_rows: u8,
    pub code_label: bool,
    pub thematic_break: bool,
    pub group_start: bool,
}

impl PendingBlockEvents {
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.spacing_rows == 0 && !self.code_label && !self.thematic_break && !self.group_start
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderCheckpoint {
    pub position: CheckpointPosition,
    pub fold_column: u16,
    pub line_source_start: Option<usize>,
    pub line_display_width: u32,
    pub block: Option<BlockId>,
    pub block_kind: CheckpointBlockKind,
    pub heading_level: u8,
    pub list_indent: u16,
    pub list_marker: ListMarkerState,
    pub list_marker_width: u8,
    pub list_body_indent: u16,
    pub list_continuation_indent: u16,
    pub quote_prefix: bool,
    pub code: CodeRenderState,
    pub table: TableRenderState,
    pub pending: PendingBlockEvents,
}

impl RenderCheckpoint {
    #[must_use]
    pub const fn line_start() -> Self {
        Self {
            position: CheckpointPosition::LineStart,
            fold_column: 0,
            line_source_start: None,
            line_display_width: 0,
            block: None,
            block_kind: CheckpointBlockKind::None,
            heading_level: 0,
            list_indent: 0,
            list_marker: ListMarkerState::None,
            list_marker_width: 0,
            list_body_indent: 0,
            list_continuation_indent: 0,
            quote_prefix: false,
            code: CodeRenderState::None,
            table: TableRenderState::None,
            pending: PendingBlockEvents {
                spacing_rows: 0,
                code_label: false,
                thematic_break: false,
                group_start: false,
            },
        }
    }

    #[must_use]
    pub const fn line_mid(source_start: usize, display_width: u32) -> Self {
        let mut checkpoint = Self::line_start();
        checkpoint.position = CheckpointPosition::LineMid;
        checkpoint.fold_column = if display_width > u16::MAX as u32 {
            u16::MAX
        } else {
            display_width as u16
        };
        checkpoint.line_source_start = Some(source_start);
        checkpoint.line_display_width = display_width;
        checkpoint
    }

    #[must_use]
    pub const fn block_inner(block: BlockId, kind: CheckpointBlockKind) -> Self {
        let mut checkpoint = Self::line_start();
        checkpoint.position = CheckpointPosition::BlockInner;
        checkpoint.block = Some(block);
        checkpoint.block_kind = kind;
        checkpoint
    }

    #[must_use]
    pub const fn has_unfinished_line(self) -> bool {
        matches!(self.position, CheckpointPosition::LineMid) && self.line_source_start.is_some()
    }
}

impl Default for RenderCheckpoint {
    fn default() -> Self {
        Self::line_start()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPlan {
    Exact(TileId),
    ReplayFrom { from: TileId, target: TileId },
    ApproximateLineStart(TileId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointMaintenancePhase {
    Drain,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointMaintenanceProgress {
    /// `Covered` means the maintenance pass completed; budget exhaustion leaves
    /// the store in the same explicit phase for the next call.
    pub completion: mdtui3::Completion,
    pub phase: Option<CheckpointMaintenancePhase>,
    pub drained_payloads: usize,
    pub retired_payloads: usize,
    pub released_payloads: usize,
    pub drain_cursor: usize,
    pub len: usize,
    pub arena: ArenaStats,
    pub compaction: Option<mdtui3::CheckpointCompactionStats>,
}

// The complete FoldView moves into maintenance. No renderer owns a second
// height tree or extracts/replaces the checkpoint index.
enum CheckpointStoreState {
    Ready(mdtui3::FoldView<RenderCheckpoint>),
    Draining {
        view: mdtui3::FoldView<RenderCheckpoint>,
        before: TileId,
        cursor: usize,
        drained: usize,
    },
    Compacting {
        task: Box<mdtui3::FoldViewCompaction<RenderCheckpoint>>,
        drained: usize,
        retired_total: usize,
    },
    Moving,
}

pub struct RenderCheckpointStore {
    state: CheckpointStoreState,
    pending_reset: Option<u64>,
}

impl RenderCheckpointStore {
    pub(crate) fn borrowed() -> Self {
        Self {
            state: CheckpointStoreState::Moving,
            pending_reset: None,
        }
    }

    pub fn new(document: &Document) -> Self {
        Self {
            pending_reset: None,
            state: CheckpointStoreState::Ready(
                mdtui3::FoldView::with_state(document, 1).expect("valid initial estimate"),
            ),
        }
    }

    pub fn view(&self) -> &mdtui3::FoldView<RenderCheckpoint> {
        match &self.state {
            CheckpointStoreState::Ready(view) | CheckpointStoreState::Draining { view, .. } => view,
            _ => panic!("layout owner is in checkpoint compaction"),
        }
    }

    pub fn view_mut(&mut self) -> Result<&mut mdtui3::FoldView<RenderCheckpoint>, LayoutError> {
        match &mut self.state {
            CheckpointStoreState::Ready(view) => Ok(view),
            _ => Err(LayoutError::OutOfOrderUpdate),
        }
    }

    pub fn heights(&self) -> &mdtui3::HeightIndex {
        match &self.state {
            CheckpointStoreState::Compacting { task, .. } => task.heights(),
            _ => self.view().heights(),
        }
    }
    pub fn reset_width(&mut self, document: &Document, estimate: u64) -> Result<(), LayoutError> {
        match &mut self.state {
            CheckpointStoreState::Ready(view) => view.reset(estimate),
            CheckpointStoreState::Draining { view, .. } => {
                // Invalidating payloads is O(1); retain their arena for later
                // paid drain/compaction instead of destroying the owner.
                view.layout_parts_mut(document)?;
                view.reset(estimate)?;
                let state = std::mem::replace(&mut self.state, CheckpointStoreState::Moving);
                let CheckpointStoreState::Draining { view, .. } = state else {
                    unreachable!()
                };
                self.state = CheckpointStoreState::Ready(view);
                Ok(())
            }
            CheckpointStoreState::Compacting { .. } => {
                self.pending_reset = Some(estimate);
                Ok(())
            }
            CheckpointStoreState::Moving => Err(LayoutError::OutOfOrderUpdate),
        }
    }

    pub fn heights_mut(
        &mut self,
        document: &Document,
    ) -> Result<&mut mdtui3::HeightIndex, LayoutError> {
        Ok(self.view_mut()?.layout_parts_mut(document)?.0)
    }

    fn ready_mut(
        &mut self,
        document: &Document,
    ) -> Result<&mut CheckpointIndex<RenderCheckpoint>, LayoutError> {
        Ok(self.view_mut()?.layout_parts_mut(document)?.1)
    }

    pub fn insert(
        &mut self,
        tile: TileId,
        checkpoint: RenderCheckpoint,
    ) -> Result<Option<RenderCheckpoint>, LayoutError> {
        if self.exact(tile) == Some(&checkpoint) {
            return Ok(None);
        }
        self.view_mut()?.insert_checkpoint(tile, checkpoint)
    }

    pub fn checkpoint_after_tile(
        &mut self,
        tile: TileId,
        checkpoint: RenderCheckpoint,
        document: &Document,
    ) -> Result<Option<RenderCheckpoint>, LayoutError> {
        let Some(next) = tile
            .0
            .checked_add(1)
            .map(TileId)
            .filter(|next| document.tile(*next).is_some())
        else {
            return Ok(None);
        };
        self.insert(next, checkpoint)
    }

    #[must_use]
    pub fn exact(&self, tile: TileId) -> Option<&RenderCheckpoint> {
        if self.pending_reset.is_some() {
            return None;
        }
        match &self.state {
            CheckpointStoreState::Ready(view) | CheckpointStoreState::Draining { view, .. } => {
                view.checkpoints().get(tile)
            }
            CheckpointStoreState::Compacting { task, .. } => task.checkpoint(tile),
            CheckpointStoreState::Moving => None,
        }
    }

    #[must_use]
    pub fn previous(&self, tile: TileId) -> Option<(TileId, &RenderCheckpoint)> {
        if self.pending_reset.is_some() {
            return None;
        }
        match &self.state {
            CheckpointStoreState::Ready(view) | CheckpointStoreState::Draining { view, .. } => {
                view.checkpoints().previous(tile)
            }
            CheckpointStoreState::Compacting { task, .. } => task.previous_checkpoint(tile),
            CheckpointStoreState::Moving => None,
        }
    }

    #[must_use]
    pub fn plan_recovery(&self, target: TileId) -> RecoveryPlan {
        if self.exact(target).is_some() {
            RecoveryPlan::Exact(target)
        } else if let Some((from, _)) = self.previous(target) {
            RecoveryPlan::ReplayFrom { from, target }
        } else {
            RecoveryPlan::ApproximateLineStart(target)
        }
    }

    #[must_use]
    pub fn token(&self, tile: TileId) -> Option<CheckpointToken> {
        if self.pending_reset.is_some() {
            return None;
        }
        match &self.state {
            CheckpointStoreState::Ready(view) | CheckpointStoreState::Draining { view, .. } => {
                view.checkpoints().token(tile)
            }
            CheckpointStoreState::Compacting { task, .. } => task.checkpoint_token(tile),
            CheckpointStoreState::Moving => None,
        }
    }

    #[must_use]
    pub fn validates(&self, token: CheckpointToken) -> bool {
        if self.pending_reset.is_some() {
            return false;
        }
        match &self.state {
            CheckpointStoreState::Ready(view) | CheckpointStoreState::Draining { view, .. } => {
                view.checkpoints().validates(token)
            }
            CheckpointStoreState::Compacting { task, .. } => task.validates_checkpoint(token),
            CheckpointStoreState::Moving => false,
        }
    }

    pub fn invalidate(
        &mut self,
        document: &Document,
        range: Range<usize>,
    ) -> Result<(), LayoutError> {
        self.ready_mut(document)?.invalidate(range)
    }
    pub fn clear(&mut self, document: &Document) -> Result<(), LayoutError> {
        self.ready_mut(document)?.clear();
        Ok(())
    }
    pub fn apply(&mut self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        self.view_mut()?.apply(document, update)
    }
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.state {
            CheckpointStoreState::Ready(view) | CheckpointStoreState::Draining { view, .. } => {
                view.checkpoints().len()
            }
            CheckpointStoreState::Compacting { task, .. } => task.checkpoint_len(),
            CheckpointStoreState::Moving => 0,
        }
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[must_use]
    pub fn arena_stats(&self) -> ArenaStats {
        match &self.state {
            CheckpointStoreState::Ready(view) | CheckpointStoreState::Draining { view, .. } => {
                view.checkpoints().arena_stats()
            }
            CheckpointStoreState::Compacting { task, .. } => arena_from_compaction(task.stats()),
            CheckpointStoreState::Moving => ArenaStats::default(),
        }
    }
    #[must_use]
    pub const fn maintenance_in_progress(&self) -> bool {
        !matches!(self.state, CheckpointStoreState::Ready(_))
    }
    #[must_use]
    pub fn maintenance_phase(&self) -> Option<CheckpointMaintenancePhase> {
        match self.state {
            CheckpointStoreState::Ready(_) => None,
            CheckpointStoreState::Draining { .. } => Some(CheckpointMaintenancePhase::Drain),
            _ => Some(CheckpointMaintenancePhase::Compact),
        }
    }
    pub fn start_maintenance(
        &mut self,
        document: &Document,
        keep: Range<usize>,
        tile_count: usize,
    ) -> Result<(), LayoutError> {
        if self.maintenance_in_progress() {
            return Ok(());
        }
        if keep.start > keep.end || keep.end > tile_count {
            return Err(LayoutError::InvalidRange);
        }
        let index = self.ready_mut(document)?;
        index.invalidate(0..keep.start)?;
        index.invalidate(keep.end..usize::MAX)?;
        let state = std::mem::replace(&mut self.state, CheckpointStoreState::Moving);
        let CheckpointStoreState::Ready(view) = state else {
            unreachable!()
        };
        self.state = CheckpointStoreState::Draining {
            view,
            before: TileId(keep.start),
            cursor: 0,
            drained: 0,
        };
        Ok(())
    }
    pub fn advance_maintenance(
        &mut self,
        document: &Document,
        budget: &mut mdtui3::Budget,
    ) -> Result<CheckpointMaintenanceProgress, LayoutError> {
        use mdtui3::Completion;
        let mut compact_stats = None;
        loop {
            let completion = match &mut self.state {
                CheckpointStoreState::Ready(_) => Completion::Covered,
                CheckpointStoreState::Draining {
                    view,
                    before,
                    cursor,
                    drained,
                } => {
                    let mut retired = Vec::new();
                    let completion = view.layout_parts_mut(document)?.1.drain_before(
                        *before,
                        cursor,
                        budget,
                        &mut retired,
                    );
                    *drained += retired.len();
                    if completion == Completion::BudgetExhausted {
                        completion
                    } else {
                        let state =
                            std::mem::replace(&mut self.state, CheckpointStoreState::Moving);
                        let CheckpointStoreState::Draining { view, drained, .. } = state else {
                            unreachable!()
                        };
                        self.state = CheckpointStoreState::Compacting {
                            task: Box::new(view.into_checkpoint_compaction()),
                            drained,
                            retired_total: 0,
                        };
                        continue;
                    }
                }
                CheckpointStoreState::Compacting {
                    task,
                    drained: _,
                    retired_total,
                } => {
                    let mut retired = Vec::new();
                    let completion = task.advance(document, budget, &mut retired)?;
                    *retired_total += retired.len();
                    compact_stats = Some(task.stats());
                    if completion == Completion::Covered {
                        let state =
                            std::mem::replace(&mut self.state, CheckpointStoreState::Moving);
                        let CheckpointStoreState::Compacting {
                            task,
                            drained,
                            retired_total,
                        } = state
                        else {
                            unreachable!()
                        };
                        match (*task).finish(document) {
                            Ok(mut view) => {
                                if let Some(estimate) = self.pending_reset.take() {
                                    view.reset(estimate)?;
                                }
                                self.state = CheckpointStoreState::Ready(view);
                                return Ok(CheckpointMaintenanceProgress {
                                    completion,
                                    phase: None,
                                    drained_payloads: drained,
                                    retired_payloads: retired_total,
                                    released_payloads: drained + retired_total,
                                    drain_cursor: 0,
                                    len: self.len(),
                                    arena: self.arena_stats(),
                                    compaction: compact_stats,
                                });
                            }
                            Err(task) => {
                                self.state = CheckpointStoreState::Compacting {
                                    task: Box::new(task),
                                    drained,
                                    retired_total,
                                };
                                return Err(LayoutError::OutOfOrderUpdate);
                            }
                        }
                    }
                    completion
                }
                CheckpointStoreState::Moving => unreachable!(),
            };
            let (drained, retired, cursor) = match &self.state {
                CheckpointStoreState::Draining {
                    drained, cursor, ..
                } => (*drained, 0, *cursor),
                CheckpointStoreState::Compacting {
                    drained,
                    retired_total,
                    ..
                } => (*drained, *retired_total, 0),
                _ => (0, 0, 0),
            };
            return Ok(CheckpointMaintenanceProgress {
                completion,
                phase: self.maintenance_phase(),
                drained_payloads: drained,
                retired_payloads: retired,
                released_payloads: drained + retired,
                drain_cursor: cursor,
                len: self.len(),
                arena: self.arena_stats(),
                compaction: compact_stats,
            });
        }
    }
}

fn arena_from_compaction(stats: mdtui3::CheckpointCompactionStats) -> ArenaStats {
    ArenaStats {
        nodes: stats.source.nodes + stats.destination.nodes + stats.scratch.nodes,
        capacity_bytes: stats.source.capacity_bytes
            + stats.destination.capacity_bytes
            + stats.scratch.capacity_bytes,
        allocation_calls: stats.source.allocation_calls
            + stats.destination.allocation_calls
            + stats.scratch.allocation_calls,
    }
}
