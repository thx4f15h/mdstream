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

enum CheckpointStoreState {
    Ready(CheckpointIndex<RenderCheckpoint>),
    Draining {
        index: CheckpointIndex<RenderCheckpoint>,
        before: TileId,
        cursor: usize,
        out: Vec<RenderCheckpoint>,
        drained: usize,
    },
    Compacting {
        task: mdtui3::CheckpointCompaction<RenderCheckpoint>,
        retired: Vec<RenderCheckpoint>,
        retired_total: usize,
        drained: usize,
    },
}

pub struct RenderCheckpointStore {
    state: CheckpointStoreState,
}

impl RenderCheckpointStore {
    pub fn new(document: &Document) -> Self {
        let mut index = CheckpointIndex::new();
        index.rebuild(document);
        Self {
            state: CheckpointStoreState::Ready(index),
        }
    }

    pub fn rebuild(&mut self, document: &Document) {
        let mut index = CheckpointIndex::new();
        index.rebuild(document);
        self.state = CheckpointStoreState::Ready(index);
    }

    fn ready_mut(&mut self) -> Result<&mut CheckpointIndex<RenderCheckpoint>, LayoutError> {
        match &mut self.state {
            CheckpointStoreState::Ready(index) => Ok(index),
            CheckpointStoreState::Draining { .. } | CheckpointStoreState::Compacting { .. } => {
                Err(LayoutError::OutOfOrderUpdate)
            }
        }
    }

    pub fn insert(
        &mut self,
        tile: TileId,
        checkpoint: RenderCheckpoint,
    ) -> Result<Option<RenderCheckpoint>, LayoutError> {
        self.ready_mut()?.insert(tile, checkpoint)
    }

    /// Save the state after `tile` only when a following tile exists.
    pub fn checkpoint_after_tile(
        &mut self,
        tile: TileId,
        checkpoint: RenderCheckpoint,
        document: &Document,
    ) -> Result<Option<RenderCheckpoint>, LayoutError> {
        let Some(next) = tile.0.checked_add(1) else {
            return Ok(None);
        };
        let next = TileId(next);
        if document.tile(next).is_none() {
            return Ok(None);
        }
        self.insert(next, checkpoint)
    }

    #[must_use]
    pub fn exact(&self, tile: TileId) -> Option<&RenderCheckpoint> {
        match &self.state {
            CheckpointStoreState::Ready(index) | CheckpointStoreState::Draining { index, .. } => {
                index.get(tile)
            }
            CheckpointStoreState::Compacting { task, .. } => task.get(tile),
        }
    }

    #[must_use]
    pub fn previous(&self, tile: TileId) -> Option<(TileId, &RenderCheckpoint)> {
        match &self.state {
            CheckpointStoreState::Ready(index) | CheckpointStoreState::Draining { index, .. } => {
                index.previous(tile)
            }
            CheckpointStoreState::Compacting { task, .. } => task.previous(tile),
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
        match &self.state {
            CheckpointStoreState::Ready(index) | CheckpointStoreState::Draining { index, .. } => {
                index.token(tile)
            }
            CheckpointStoreState::Compacting { task, .. } => task.token(tile),
        }
    }

    #[must_use]
    pub fn validates(&self, token: CheckpointToken) -> bool {
        match &self.state {
            CheckpointStoreState::Ready(index) | CheckpointStoreState::Draining { index, .. } => {
                index.validates(token)
            }
            CheckpointStoreState::Compacting { task, .. } => task.validates(token),
        }
    }

    pub fn invalidate(&mut self, range: Range<usize>) -> Result<(), LayoutError> {
        self.ready_mut()?.invalidate(range)
    }

    pub fn clear(&mut self) {
        if let CheckpointStoreState::Ready(index) = &mut self.state {
            index.clear();
        }
    }

    pub fn apply(&mut self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        self.ready_mut()?.apply(document, update)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        match &self.state {
            CheckpointStoreState::Ready(index) | CheckpointStoreState::Draining { index, .. } => {
                index.len()
            }
            CheckpointStoreState::Compacting { task, .. } => task.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn arena_stats(&self) -> ArenaStats {
        match &self.state {
            CheckpointStoreState::Ready(index) | CheckpointStoreState::Draining { index, .. } => {
                index.arena_stats()
            }
            CheckpointStoreState::Compacting { task, .. } => {
                let stats = task.stats();
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
        }
    }

    #[must_use]
    pub const fn maintenance_in_progress(&self) -> bool {
        !matches!(&self.state, CheckpointStoreState::Ready(_))
    }

    #[must_use]
    pub fn maintenance_phase(&self) -> Option<CheckpointMaintenancePhase> {
        match &self.state {
            CheckpointStoreState::Ready(_) => None,
            CheckpointStoreState::Draining { .. } => Some(CheckpointMaintenancePhase::Drain),
            CheckpointStoreState::Compacting { .. } => Some(CheckpointMaintenancePhase::Compact),
        }
    }

    /// Invalidate checks outside `keep`, then begin the budgeted drain and
    /// compaction pipeline. The upper range intentionally spans the index arena,
    /// not only `document.tile_count()`, so stale payloads beyond the current EOF
    /// are not hidden from compaction.
    pub fn start_maintenance(
        &mut self,
        keep: Range<usize>,
        tile_count: usize,
    ) -> Result<(), LayoutError> {
        if self.maintenance_in_progress() {
            return Ok(());
        }
        if keep.start > keep.end || keep.end > tile_count {
            return Err(LayoutError::InvalidRange);
        }
        let state = std::mem::replace(
            &mut self.state,
            CheckpointStoreState::Ready(CheckpointIndex::new()),
        );
        let CheckpointStoreState::Ready(mut index) = state else {
            unreachable!("checked ready maintenance state");
        };
        index.invalidate(0..keep.start)?;
        index.invalidate(keep.end..usize::MAX)?;
        self.state = CheckpointStoreState::Draining {
            index,
            before: TileId(keep.start),
            cursor: 0,
            out: Vec::new(),
            drained: 0,
        };
        Ok(())
    }

    pub fn advance_maintenance(
        &mut self,
        budget: &mut mdtui3::Budget,
    ) -> Result<CheckpointMaintenanceProgress, LayoutError> {
        use mdtui3::{Completion, WorkLimit};

        match &mut self.state {
            CheckpointStoreState::Ready(_) => Ok(self.completed_progress(None)),
            CheckpointStoreState::Draining { .. } => {
                let CheckpointStoreState::Draining {
                    index,
                    before,
                    cursor,
                    out,
                    drained,
                } = &mut self.state
                else {
                    unreachable!()
                };
                let completion = index.drain_before(*before, cursor, budget, out);
                let released_now = out.len();
                *drained = drained.saturating_add(released_now);
                out.clear();
                if completion == Completion::BudgetExhausted {
                    return Ok(CheckpointMaintenanceProgress {
                        completion,
                        phase: Some(CheckpointMaintenancePhase::Drain),
                        drained_payloads: *drained,
                        retired_payloads: 0,
                        released_payloads: *drained,
                        drain_cursor: *cursor,
                        len: index.len(),
                        arena: index.arena_stats(),
                        compaction: None,
                    });
                }
                *cursor = 0;
                let state = std::mem::replace(
                    &mut self.state,
                    CheckpointStoreState::Ready(CheckpointIndex::new()),
                );
                let CheckpointStoreState::Draining {
                    index,
                    drained,
                    cursor,
                    ..
                } = state
                else {
                    unreachable!()
                };
                debug_assert_eq!(cursor, 0);
                self.state = CheckpointStoreState::Compacting {
                    task: index.into_compaction(),
                    retired: Vec::new(),
                    retired_total: 0,
                    drained,
                };
                if !budget.charge(WorkLimit {
                    queries: 1,
                    ..WorkLimit::default()
                }) {
                    return Ok(self.in_progress_progress());
                }
                self.advance_compaction(budget)
            }
            CheckpointStoreState::Compacting { .. } => self.advance_compaction(budget),
        }
    }

    fn advance_compaction(
        &mut self,
        budget: &mut mdtui3::Budget,
    ) -> Result<CheckpointMaintenanceProgress, LayoutError> {
        use mdtui3::Completion;

        let CheckpointStoreState::Compacting {
            task,
            retired,
            retired_total,
            drained,
        } = &mut self.state
        else {
            unreachable!("compaction state")
        };
        let before_retired = retired.len();
        let completion = task.advance(budget, retired);
        let released_now = retired.len().saturating_sub(before_retired);
        *retired_total = retired_total.saturating_add(released_now);
        retired.clear();
        if completion == Completion::BudgetExhausted {
            let task_stats = task.stats();
            return Ok(CheckpointMaintenanceProgress {
                completion,
                phase: Some(CheckpointMaintenancePhase::Compact),
                drained_payloads: *drained,
                retired_payloads: *retired_total,
                released_payloads: drained.saturating_add(*retired_total),
                drain_cursor: 0,
                len: task.len(),
                arena: arena_from_compaction(task_stats),
                compaction: Some(task_stats),
            });
        }
        let stats = task.stats();
        let state = std::mem::replace(
            &mut self.state,
            CheckpointStoreState::Ready(CheckpointIndex::new()),
        );
        let CheckpointStoreState::Compacting {
            task,
            drained,
            retired_total,
            ..
        } = state
        else {
            unreachable!()
        };
        match task.finish() {
            Ok(index) => {
                let len = index.len();
                let arena = index.arena_stats();
                self.state = CheckpointStoreState::Ready(index);
                Ok(CheckpointMaintenanceProgress {
                    completion: Completion::Covered,
                    phase: None,
                    drained_payloads: drained,
                    retired_payloads: retired_total,
                    released_payloads: drained.saturating_add(retired_total),
                    drain_cursor: 0,
                    len,
                    arena,
                    compaction: Some(stats),
                })
            }
            Err(task) => {
                self.state = CheckpointStoreState::Compacting {
                    task,
                    retired: Vec::new(),
                    retired_total,
                    drained,
                };
                Err(LayoutError::OutOfOrderUpdate)
            }
        }
    }

    fn in_progress_progress(&self) -> CheckpointMaintenanceProgress {
        use mdtui3::Completion;
        match &self.state {
            CheckpointStoreState::Ready(_) => self.completed_progress(None),
            CheckpointStoreState::Draining {
                index,
                cursor,
                drained,
                ..
            } => CheckpointMaintenanceProgress {
                completion: Completion::BudgetExhausted,
                phase: Some(CheckpointMaintenancePhase::Drain),
                drained_payloads: *drained,
                retired_payloads: 0,
                released_payloads: 0,
                drain_cursor: *cursor,
                len: index.len(),
                arena: index.arena_stats(),
                compaction: None,
            },
            CheckpointStoreState::Compacting {
                task,
                drained,
                retired_total,
                ..
            } => CheckpointMaintenanceProgress {
                completion: Completion::BudgetExhausted,
                phase: Some(CheckpointMaintenancePhase::Compact),
                drained_payloads: *drained,
                retired_payloads: *retired_total,
                released_payloads: 0,
                drain_cursor: 0,
                len: task.len(),
                arena: self.arena_stats(),
                compaction: Some(task.stats()),
            },
        }
    }

    fn completed_progress(
        &self,
        stats: Option<mdtui3::CheckpointCompactionStats>,
    ) -> CheckpointMaintenanceProgress {
        use mdtui3::Completion;
        CheckpointMaintenanceProgress {
            completion: Completion::Covered,
            phase: None,
            drained_payloads: 0,
            retired_payloads: 0,
            released_payloads: 0,
            drain_cursor: 0,
            len: self.len(),
            arena: self.arena_stats(),
            compaction: stats,
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
