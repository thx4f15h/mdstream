use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mdtui3::{Budget, Document, LinkId, TileId, Update, WorkLimit};

use crate::cli::LinkMode;
use crate::external::{
    ExternalOpenOutcome, ExternalOpenStatus, ExternalOpener, ExternalTarget, SystemExternalOpener,
    command_for_external,
};
use crate::fragment::{AnchorBuildStatus, FragmentActivation};
use crate::history::HistoryPosition;
use crate::links::LinkTable;
use crate::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameError, FirstFrameStatus};
use crate::render::frame::{
    FrameCommit, FrameError, FrameMetrics, FrameResize, FrameRuntime, HistoryRestoreStatus,
    SidebarLayout,
};
use crate::render::viewport::{
    EofCalibrationStatus, ViewportError, ViewportFrame, ViewportFrameStatus,
};
use crate::screen::Screen;

pub const READ_CHUNK_BYTES: usize = 64 * 1024;
pub const PRE_READ_BATCH_BYTES: usize = 32 * 1024;
pub const MAX_PRE_READ_BATCHES: usize = 9;
pub const MAX_PRE_READ_BYTES: usize = PRE_READ_BATCH_BYTES * MAX_PRE_READ_BATCHES;
pub const FIRST_FRAME_FALLBACK_DELAY: Duration = Duration::from_millis(16);
pub const MAX_UPDATES_PER_FRAME: usize = 32;
pub const MAX_BATCH_DELAY: Duration = Duration::from_millis(16);
pub const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(20);
pub const ANCHOR_POLL_INTERVAL: Duration = Duration::from_millis(1);
pub const FRAME_WORK: mdtui3::WorkLimit = mdtui3::WorkLimit {
    queries: 4_096,
    tiles: 512,
    bytes: 256 * 1024,
    objects: 4_096,
    recovery: 256,
    dependencies: 4_096,
};
pub const ANCHOR_WORK: mdtui3::WorkLimit = mdtui3::WorkLimit {
    queries: 128,
    tiles: 1,
    bytes: 16 * 1024,
    objects: 1_024,
    recovery: 1,
    dependencies: 0,
};
const PROBE_WORK: mdtui3::WorkLimit = mdtui3::WorkLimit::unlimited();

#[derive(Debug)]
pub enum LoadEvent {
    Data(Vec<u8>),
    Eof,
    Error(io::Error),
}

pub struct Reader {
    cancelled: Arc<AtomicBool>,
    receiver: Option<Receiver<LoadEvent>>,
    handle: Option<JoinHandle<()>>,
}

impl Reader {
    pub fn spawn<R>(mut reader: R) -> io::Result<Self>
    where
        R: Read + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let handle = thread::Builder::new()
            .name("mdtui-stream-reader".to_owned())
            .spawn(move || {
                let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
                while !worker_cancelled.load(Ordering::Acquire) {
                    match reader.read(&mut buffer) {
                        Ok(0) => {
                            let _ = sender.send(LoadEvent::Eof);
                            break;
                        }
                        Ok(count) => {
                            let data = buffer[..count].to_vec();
                            if sender.send(LoadEvent::Data(data)).is_err() {
                                break;
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            let _ = sender.send(LoadEvent::Error(error));
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            cancelled,
            receiver: Some(receiver),
            handle: Some(handle),
        })
    }

    pub fn try_recv(&self) -> Result<LoadEvent, TryRecvError> {
        self.receiver
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<LoadEvent, RecvTimeoutError> {
        self.receiver
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn close(&mut self) -> Result<(), StreamError> {
        self.cancelled.store(true, Ordering::Release);
        // Disconnect a blocked send. Arbitrary Read implementations cannot be
        // interrupted safely: detach an unfinished worker rather than waiting
        // for EOF during q/Drop. It stops after its current read returns.
        self.receiver.take();
        if let Some(handle) = self.handle.take().filter(JoinHandle::is_finished) {
            handle.join().map_err(|_| StreamError::ReaderPanicked)?;
        }
        Ok(())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestResult {
    FirstFrame,
    PreReading,
    Applied,
    Empty,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamMetrics {
    /// Bytes delivered to `ingest_data`, counted exactly once, including bytes
    /// buffered by first-frame pre-reading and drained after the first frame.
    pub read_bytes: usize,
    /// Non-empty `Update`s actually applied to the viewport.
    pub updates: usize,
    pub frames: usize,
    pub diff_rows: usize,
    pub recovery_tiles: usize,
    pub output_bytes: usize,
    pub layout_budget: WorkLimit,
    pub last_frame: Option<FrameMetrics>,
}

impl StreamMetrics {
    fn record_frame(&mut self, frame: FrameMetrics) {
        self.frames = self.frames.saturating_add(1);
        self.diff_rows = self.diff_rows.saturating_add(frame.diff_rows);
        self.recovery_tiles = self.recovery_tiles.saturating_add(frame.recovery_tiles);
        self.output_bytes = self.output_bytes.saturating_add(frame.output_bytes);
        accumulate_work_limit(&mut self.layout_budget, frame.layout_budget);
        self.last_frame = Some(frame);
    }
}

fn accumulate_work_limit(total: &mut WorkLimit, used: WorkLimit) {
    total.queries = total.queries.saturating_add(used.queries);
    total.tiles = total.tiles.saturating_add(used.tiles);
    total.bytes = total.bytes.saturating_add(used.bytes);
    total.objects = total.objects.saturating_add(used.objects);
    total.recovery = total.recovery.saturating_add(used.recovery);
    total.dependencies = total.dependencies.saturating_add(used.dependencies);
}

#[derive(Debug)]
pub enum StreamError {
    Io(io::Error),
    Frame(FrameError),
    FirstFrame(FirstFrameError),
    InputFinished,
    MissingCalibratedFrame,
    ReaderPanicked,
}

impl fmt::Display for StreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Frame(error) => error.fmt(formatter),
            Self::FirstFrame(error) => error.fmt(formatter),
            Self::InputFinished => formatter.write_str("stream input is already finished"),
            Self::MissingCalibratedFrame => {
                formatter.write_str("EOF calibration completed without a frame")
            }
            Self::ReaderPanicked => formatter.write_str("stream reader thread panicked"),
        }
    }
}

impl std::error::Error for StreamError {}

impl From<FrameError> for StreamError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<FirstFrameError> for StreamError {
    fn from(error: FirstFrameError) -> Self {
        Self::FirstFrame(error)
    }
}

impl From<io::Error> for StreamError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Probe whether the current document has at least one TUI-drawable row.
///
/// This uses an isolated `FirstFrame` and never advances the live
/// `ViewportState`, writes a frame, or calls `Document::finish`.
pub fn probe_has_drawable_lines(
    document: &Document,
    width: u16,
    budget: &mut Budget,
) -> Result<bool, StreamError> {
    if width == 0 {
        return Ok(false);
    }
    let mut first = FirstFrame::new(
        document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: document.tile_count().max(1),
            width,
            row_limit: 1,
        },
    )?;
    if !first
        .lines()
        .iter()
        .any(|line| !line.cells.is_empty() || line.source.is_some())
    {
        let _ = first.advance(document, budget)?;
    }
    Ok(first
        .lines()
        .iter()
        .any(|line| !line.cells.is_empty() || line.source.is_some()))
}

fn scratch_first_frame(
    document: &Document,
    width: u16,
    rows: u16,
) -> Result<Option<ViewportFrame>, StreamError> {
    if rows == 0 || document.tile_count() == 0 {
        return Ok(None);
    }
    let mut first = FirstFrame::new(
        document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: document.tile_count(),
            width,
            row_limit: u64::from(rows),
        },
    )?;
    let mut budget = Budget::new(mdtui3::WorkLimit::unlimited());
    loop {
        let step = first.advance(document, &mut budget)?;
        if first.lines().len() >= usize::from(rows)
            || matches!(
                step.status,
                FirstFrameStatus::Covered
                    | FirstFrameStatus::EndOfDocument
                    | FirstFrameStatus::BudgetExhausted
                    | FirstFrameStatus::PendingScan
            )
        {
            break;
        }
    }
    if first.lines().is_empty() {
        return Ok(None);
    }
    let lines = first.lines().to_vec();
    let rows = u16::try_from(lines.len()).unwrap_or(rows).min(rows);
    Ok(Some(ViewportFrame {
        document: document.id(),
        revision: document.revision(),
        lines,
        top: crate::render::viewport::TopPosition::Empty,
        rows,
        status: ViewportFrameStatus::Ready,
        stats: crate::render::viewport::ViewportFrameStats {
            layout_budget: budget.used(),
            ..Default::default()
        },
    }))
}

#[derive(Debug, Default)]
struct PreReadState {
    buffer: VecDeque<u8>,
    bytes: usize,
    started_at: Option<Duration>,
    done: bool,
}

impl PreReadState {
    fn push(&mut self, bytes: &[u8], now: Duration) {
        if !bytes.is_empty() && self.started_at.is_none() {
            self.started_at = Some(now);
        }
        self.buffer.extend(bytes);
    }

    fn take(&mut self, count: usize) -> Vec<u8> {
        self.buffer.drain(..count).collect()
    }

    fn fallback_due(&self, now: Duration) -> bool {
        self.bytes > 0
            && !self.done
            && self
                .started_at
                .is_some_and(|started| now.saturating_sub(started) >= FIRST_FRAME_FALLBACK_DELAY)
    }

    fn remaining_fallback(&self, now: Duration) -> Duration {
        self.started_at.map_or(Duration::ZERO, |started| {
            FIRST_FRAME_FALLBACK_DELAY.saturating_sub(now.saturating_sub(started))
        })
    }

    fn drain_all(&mut self) -> Vec<u8> {
        self.buffer.drain(..).collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FrameBatch {
    updates: usize,
    first_update_at: Option<Duration>,
}

impl FrameBatch {
    fn record_update(&mut self, now: Duration) {
        if self.updates == 0 {
            self.first_update_at = Some(now);
        }
        self.updates = self.updates.saturating_add(1);
    }

    fn due(self, now: Duration) -> bool {
        self.updates >= MAX_UPDATES_PER_FRAME
            || (self.updates > 0
                && self
                    .first_update_at
                    .is_some_and(|first| now.saturating_sub(first) >= MAX_BATCH_DELAY))
    }

    fn remaining(self, now: Duration) -> Duration {
        if self.updates == 0 {
            return Duration::ZERO;
        }
        if self.updates >= MAX_UPDATES_PER_FRAME {
            return Duration::ZERO;
        }
        self.first_update_at.map_or(Duration::ZERO, |first| {
            MAX_BATCH_DELAY.saturating_sub(now.saturating_sub(first))
        })
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct StreamEngine {
    runtime: FrameRuntime,
    pre_read: PreReadState,
    pending: FrameBatch,
    first_frame_pending: bool,
    first_frame_empty: bool,
    first_frame_done: bool,
    empty_frame_requested: bool,
    scratch_frame_requested: bool,
    force_render: bool,
    input_finished: bool,
    calibration_pending: bool,
    calibrated_frame: Option<ViewportFrame>,
    external_opener: Box<dyn ExternalOpener>,
    last_external_open: Option<ExternalOpenOutcome>,
    metrics: StreamMetrics,
}

impl StreamEngine {
    pub fn new(runtime: FrameRuntime) -> Self {
        Self::with_external_opener(runtime, Box::new(SystemExternalOpener))
    }

    pub fn with_external_opener(
        runtime: FrameRuntime,
        external_opener: Box<dyn ExternalOpener>,
    ) -> Self {
        Self {
            runtime,
            pre_read: PreReadState::default(),
            pending: FrameBatch::default(),
            first_frame_pending: false,
            first_frame_empty: false,
            first_frame_done: false,
            empty_frame_requested: false,
            scratch_frame_requested: false,
            force_render: false,
            input_finished: false,
            calibration_pending: false,
            calibrated_frame: None,
            external_opener,
            last_external_open: None,
            metrics: StreamMetrics::default(),
        }
    }

    #[must_use]
    pub fn runtime(&self) -> &FrameRuntime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut FrameRuntime {
        &mut self.runtime
    }

    #[must_use]
    pub fn document(&self) -> &Document {
        self.runtime.document()
    }

    #[must_use]
    pub fn screen(&self) -> &Screen {
        self.runtime.screen()
    }

    #[must_use]
    pub const fn first_frame_done(&self) -> bool {
        self.first_frame_done
    }

    #[must_use]
    pub const fn input_finished(&self) -> bool {
        self.input_finished
    }

    #[must_use]
    pub const fn calibration_pending(&self) -> bool {
        self.calibration_pending
    }

    #[must_use]
    pub const fn pending_updates(&self) -> usize {
        self.pending.updates
    }

    #[must_use]
    pub const fn pre_read_bytes(&self) -> usize {
        self.pre_read.bytes
    }

    #[must_use]
    pub const fn pre_read_batches(&self) -> usize {
        self.pre_read.bytes / PRE_READ_BATCH_BYTES
    }

    #[must_use]
    pub fn queued_bytes(&self) -> usize {
        self.pre_read.buffer.len()
    }

    #[must_use]
    pub const fn metrics(&self) -> StreamMetrics {
        self.metrics
    }

    #[must_use]
    pub const fn last_frame_metrics(&self) -> Option<FrameMetrics> {
        self.metrics.last_frame
    }

    pub fn request_render(&mut self) {
        self.force_render = true;
    }

    #[must_use]
    pub fn navigation_position(&self) -> Option<HistoryPosition> {
        self.runtime.navigation_position()
    }

    pub fn restore_navigation_position(
        &mut self,
        position: HistoryPosition,
    ) -> Result<HistoryRestoreStatus, StreamError> {
        let mut budget = Budget::new(FRAME_WORK);
        self.restore_navigation_position_with_budget(position, &mut budget)
    }

    pub fn restore_navigation_position_with_budget(
        &mut self,
        position: HistoryPosition,
        budget: &mut Budget,
    ) -> Result<HistoryRestoreStatus, StreamError> {
        let restored = self.runtime.restore_navigation_position(position, budget)?;
        if restored.moved() {
            self.calibrated_frame = None;
            self.force_render = true;
        }
        Ok(restored)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<FrameResize, StreamError> {
        let resize = self.runtime.resize(rows, cols)?;
        if resize.height_changed || resize.width_changed {
            self.calibrated_frame = None;
            self.force_render = true;
        }
        if resize.width_changed && self.input_finished {
            self.calibration_pending = true;
        }
        Ok(resize)
    }

    pub fn set_content_width(
        &mut self,
        width: u16,
    ) -> Result<crate::render::viewport::ContentWidthChange, StreamError> {
        let change = self.runtime.set_content_width(width)?;
        if change.changed {
            self.calibrated_frame = None;
            self.force_render = true;
            if self.input_finished {
                self.calibration_pending = true;
            }
        }
        Ok(change)
    }

    pub fn set_sidebar_visible(&mut self, visible: bool) -> Result<SidebarLayout, StreamError> {
        let layout = self.runtime.set_sidebar_visible(visible)?;
        if layout.changed {
            self.calibrated_frame = None;
            self.force_render = true;
            if self.input_finished {
                self.calibration_pending = true;
            }
        }
        Ok(layout)
    }

    pub fn move_sidebar_selection(&mut self, delta: isize) -> bool {
        let changed = self.runtime.sidebar_move_selection(delta);
        if changed {
            self.force_render = true;
        }
        changed
    }

    pub fn sidebar_selection_home(&mut self) -> bool {
        let changed = self.runtime.sidebar_selection_home();
        if changed {
            self.force_render = true;
        }
        changed
    }

    pub fn sidebar_selection_end(&mut self) -> bool {
        let changed = self.runtime.sidebar_selection_end();
        if changed {
            self.force_render = true;
        }
        changed
    }

    pub fn activate_sidebar_selection(&mut self) -> Result<bool, StreamError> {
        let mut budget = Budget::new(FRAME_WORK);
        let moved = self.runtime.activate_sidebar_selection(&mut budget)?;
        if moved || self.runtime.has_pending_jump() {
            self.calibrated_frame = None;
            self.force_render = true;
        }
        Ok(moved)
    }

    #[must_use]
    pub fn link_table(&self) -> Option<&LinkTable> {
        self.runtime.link_table()
    }

    #[must_use]
    pub const fn link_focus(&self) -> Option<LinkId> {
        self.runtime.link_focus()
    }

    #[must_use]
    pub fn has_visible_links(&self) -> Option<bool> {
        self.runtime.link_table().map(|table| !table.is_empty())
    }

    pub fn enter_link_focus(&mut self, forward: bool) -> Option<LinkId> {
        let focused = self.runtime.enter_link_focus(forward);
        if focused.is_some() {
            self.force_render = true;
        }
        focused
    }

    pub fn cycle_link_focus(&mut self, forward: bool) -> bool {
        let changed = self.runtime.cycle_link_focus(forward);
        if changed {
            self.force_render = true;
        }
        changed
    }

    pub fn clear_link_focus(&mut self) -> bool {
        let changed = self.runtime.clear_link_focus();
        if changed {
            self.force_render = true;
        }
        changed
    }

    pub fn activate_focused_link(&mut self) -> Result<FragmentActivation, StreamError> {
        let mut budget = Budget::new(FRAME_WORK);
        self.activate_focused_link_with_budget(&mut budget)
    }

    pub fn activate_focused_link_with_budget(
        &mut self,
        budget: &mut Budget,
    ) -> Result<FragmentActivation, StreamError> {
        let activation = self.runtime.activate_focused_link(budget)?;
        self.last_external_open = if activation.status
            == crate::fragment::FragmentActivationStatus::External
            && self.runtime.link_mode() == LinkMode::OpenExternal
        {
            Some(self.open_external_link())
        } else {
            None
        };
        let notice = self
            .last_external_open
            .map(|outcome| outcome.status.message())
            .or_else(|| {
                activation
                    .status
                    .shows_notice()
                    .then(|| activation.status.message())
            });
        let notice_changed = self.runtime.set_notice(notice);
        if activation.moved || notice_changed || self.runtime.has_pending_jump() {
            self.calibrated_frame = None;
            self.force_render = true;
        }
        Ok(activation)
    }

    #[must_use]
    pub fn notice(&self) -> Option<&'static str> {
        self.runtime.notice()
    }

    #[must_use]
    pub const fn last_external_open(&self) -> Option<ExternalOpenOutcome> {
        self.last_external_open
    }

    fn open_external_link(&mut self) -> ExternalOpenOutcome {
        let Some(id) = self.runtime.link_focus() else {
            return ExternalOpenOutcome::new(ExternalOpenStatus::Rejected);
        };
        let target = match ExternalTarget::from_document(self.runtime.document(), id) {
            Ok(target) => target,
            Err(error) => {
                return ExternalOpenOutcome::new(ExternalOpenStatus::from_target_error(error));
            }
        };
        if command_for_external(&target).is_err() {
            return ExternalOpenOutcome::new(ExternalOpenStatus::Unsupported);
        }
        self.external_opener.open(&target)
    }

    pub fn advance_anchor_index(
        &mut self,
        budget: &mut Budget,
    ) -> Result<AnchorBuildStatus, StreamError> {
        Ok(self.runtime.advance_anchor_index(budget)?)
    }

    #[must_use]
    pub fn anchor_index_started(&self) -> bool {
        self.runtime.anchor_index_started()
    }

    #[must_use]
    pub fn anchor_indexed_headings(&self) -> Option<usize> {
        self.runtime.anchor_indexed_headings()
    }

    #[must_use]
    pub fn anchor_index_complete(&self) -> bool {
        self.runtime.anchor_index_complete()
    }

    #[must_use]
    pub fn anchor_build_pending(&self) -> bool {
        self.runtime.document().is_finished() && !self.runtime.anchor_index_complete()
    }

    pub fn ingest_data(
        &mut self,
        bytes: &[u8],
        now: Duration,
    ) -> Result<IngestResult, StreamError> {
        if self.input_finished {
            return Err(StreamError::InputFinished);
        }
        self.metrics.read_bytes = self.metrics.read_bytes.saturating_add(bytes.len());
        if !self.first_frame_done {
            self.pre_read.push(bytes, now);
            if let Some(drawable) = self.advance_pre_read(now)? {
                self.first_frame_pending = true;
                self.first_frame_empty = !drawable;
                return Ok(IngestResult::FirstFrame);
            }
            return Ok(IngestResult::PreReading);
        }
        debug_assert!(self.pre_read.buffer.is_empty());
        let update = self.runtime.append_bytes(bytes)?;
        let Some(update) = update else {
            return Ok(IngestResult::Empty);
        };
        self.apply_update(&update)?;
        self.pending.record_update(now);
        Ok(IngestResult::Applied)
    }

    fn advance_pre_read(&mut self, now: Duration) -> Result<Option<bool>, StreamError> {
        loop {
            if self.pre_read.bytes >= MAX_PRE_READ_BYTES {
                return Ok(Some(false));
            }
            if self.pre_read.buffer.is_empty() {
                return Ok(self.pre_read.fallback_due(now).then_some(false));
            }
            let remaining_budget = MAX_PRE_READ_BYTES.saturating_sub(self.pre_read.bytes);
            let take = self
                .pre_read
                .buffer
                .len()
                .min(PRE_READ_BATCH_BYTES)
                .min(remaining_budget);
            let chunk = self.pre_read.take(take);
            if let Some(update) = self.runtime.append_bytes(&chunk)? {
                self.apply_update(&update)?;
            }
            self.pre_read.bytes = self.pre_read.bytes.saturating_add(take);
            let mut probe_budget = Budget::new(PROBE_WORK);
            if probe_has_drawable_lines(
                self.runtime.document(),
                self.runtime.geometry().content_width,
                &mut probe_budget,
            )? {
                return Ok(Some(true));
            }
        }
    }

    fn drain_pre_read_after_first_frame(&mut self, now: Duration) -> Result<(), StreamError> {
        if self.pre_read.done {
            return Ok(());
        }
        let remainder = self.pre_read.drain_all();
        if !remainder.is_empty()
            && let Some(update) = self.runtime.append_bytes(&remainder)?
        {
            self.apply_update(&update)?;
            self.pending.record_update(now);
        }
        self.pre_read.done = true;
        Ok(())
    }

    pub fn finish_input(&mut self, _now: Duration) -> Result<bool, StreamError> {
        if self.input_finished {
            return Ok(false);
        }
        if !self.pre_read.done {
            let remainder = self.pre_read.drain_all();
            if !remainder.is_empty()
                && let Some(update) = self.runtime.append_bytes(&remainder)?
            {
                self.apply_update(&update)?;
            }
            self.pre_read.done = true;
        }
        if let Some(update) = self.runtime.finish_input()? {
            self.apply_update(&update)?;
        }
        self.input_finished = true;
        self.calibration_pending = true;
        self.calibrated_frame = None;
        if !self.first_frame_done {
            self.first_frame_pending = true;
        }
        Ok(true)
    }

    pub fn advance_calibration(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<ViewportFrame>, StreamError> {
        if !self.calibration_pending {
            return Ok(None);
        }
        let progress = match self.runtime.advance_eof_calibration(budget) {
            Ok(progress) => progress,
            Err(FrameError::Viewport(ViewportError::IncompleteFrame)) => {
                self.calibration_pending = false;
                let bounded_probe = self.runtime.document().source().len() <= MAX_PRE_READ_BYTES
                    && probe_has_drawable_lines(
                        self.runtime.document(),
                        self.runtime.geometry().content_width,
                        &mut Budget::new(PROBE_WORK),
                    )?;
                if bounded_probe {
                    self.scratch_frame_requested = true;
                } else {
                    self.empty_frame_requested = true;
                }
                self.force_render = true;
                if !self.first_frame_done {
                    self.first_frame_pending = true;
                    self.first_frame_empty = true;
                }
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        if progress.status != EofCalibrationStatus::Calibrated {
            return Ok(None);
        }
        let frame = progress.frame.ok_or(StreamError::MissingCalibratedFrame)?;
        self.calibrated_frame = Some(frame.clone());
        Ok(Some(frame))
    }

    pub fn due(&mut self, now: Duration) -> bool {
        if !self.first_frame_done && !self.first_frame_pending && self.pre_read.fallback_due(now) {
            self.first_frame_pending = true;
            self.first_frame_empty = true;
        }
        self.calibrated_frame.is_some()
            || (!self.first_frame_done && self.first_frame_pending && !self.calibration_pending)
            || self.force_render
            || (!self.calibration_pending && self.pending.due(now))
    }

    pub fn poll_timeout(&self, now: Duration) -> Duration {
        if self.calibrated_frame.is_some()
            || (!self.first_frame_done && self.first_frame_pending && !self.calibration_pending)
        {
            return Duration::ZERO;
        }
        if self.calibration_pending {
            return Duration::from_millis(1);
        }
        if !self.first_frame_done && self.pre_read.bytes > 0 {
            return self.pre_read.remaining_fallback(now);
        }
        if self.force_render || self.pending.updates >= MAX_UPDATES_PER_FRAME {
            return Duration::ZERO;
        }
        if self.pending.updates > 0 {
            return self.pending.remaining(now);
        }
        if self.anchor_build_pending() {
            return ANCHOR_POLL_INTERVAL;
        }
        IDLE_POLL_INTERVAL
    }

    pub fn render_due<W: io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        budget: &mut Budget,
        now: Duration,
    ) -> Result<Option<FrameCommit>, StreamError> {
        if let Some(frame) = self.calibrated_frame.take() {
            let commit = self.runtime.commit_frame(writer, &frame)?;
            if commit.committed {
                self.first_frame_pending = false;
                self.first_frame_empty = false;
                self.first_frame_done = true;
                self.empty_frame_requested = false;
                self.scratch_frame_requested = false;
                self.calibration_pending = false;
                self.force_render = false;
                self.pending.reset();
                self.drain_pre_read_after_first_frame(now)?;
                self.record_commit(&commit);
                return Ok(Some(commit));
            }
            self.calibrated_frame = Some(frame);
            return Ok(None);
        }

        if !self.first_frame_done {
            if !self.first_frame_pending || self.calibration_pending {
                return Ok(None);
            }
            let commit = if self.scratch_frame_requested {
                if let Some(frame) = scratch_first_frame(
                    self.runtime.document(),
                    self.runtime.geometry().content_width,
                    self.runtime.geometry().rows,
                )? {
                    self.runtime.commit_frame(writer, &frame)?
                } else {
                    self.runtime.commit_empty_frame(writer)?
                }
            } else if self.first_frame_empty {
                self.runtime.commit_empty_frame(writer)?
            } else {
                match self.runtime.frame_to(writer, budget) {
                    Ok(commit) => commit,
                    Err(FrameError::Viewport(ViewportError::IncompleteFrame)) => {
                        if let Some(frame) = scratch_first_frame(
                            self.runtime.document(),
                            self.runtime.geometry().content_width,
                            self.runtime.geometry().rows,
                        )? {
                            self.runtime.commit_frame(writer, &frame)?
                        } else {
                            self.first_frame_empty = true;
                            self.runtime.commit_empty_frame(writer)?
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            if commit.committed {
                self.first_frame_pending = false;
                self.first_frame_empty = false;
                self.first_frame_done = true;
                self.empty_frame_requested = false;
                self.scratch_frame_requested = false;
                self.pending.reset();
                self.drain_pre_read_after_first_frame(now)?;
                self.record_commit(&commit);
                return Ok(Some(commit));
            }
            return Ok(None);
        }

        if (self.calibration_pending && !self.force_render)
            || !(self.force_render || self.pending.due(now))
        {
            return Ok(None);
        }
        let commit = if self.scratch_frame_requested {
            if let Some(frame) = scratch_first_frame(
                self.runtime.document(),
                self.runtime.geometry().content_width,
                self.runtime.geometry().rows,
            )? {
                self.runtime.commit_frame(writer, &frame)?
            } else {
                self.runtime.commit_empty_frame(writer)?
            }
        } else if self.empty_frame_requested {
            self.runtime.commit_empty_frame(writer)?
        } else {
            self.runtime.frame_to(writer, budget)?
        };
        if commit.committed {
            self.empty_frame_requested = false;
            self.scratch_frame_requested = false;
            self.force_render = false;
            self.pending.reset();
            self.record_commit(&commit);
            return Ok(Some(commit));
        }
        Ok(None)
    }

    fn apply_update(&mut self, update: &Update) -> Result<(), StreamError> {
        self.runtime.apply_update(update)?;
        self.metrics.updates = self.metrics.updates.saturating_add(1);
        Ok(())
    }

    fn record_commit(&mut self, commit: &FrameCommit) {
        self.metrics.record_frame(commit.metrics);
        if self.runtime.sidebar_pending() {
            self.force_render = true;
        }
    }
}
