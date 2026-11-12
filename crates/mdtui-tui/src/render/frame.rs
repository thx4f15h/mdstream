use std::fmt;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use mdtui3::{
    Anchor, Budget, Completion, Document, FragmentStep, HeadingAnchors, HeadingId, LayoutError,
    LinkId, NavAction, Navigator, Update, Utf8Document, WorkLimit,
};

use crate::cli::LinkMode;
use crate::fragment::{
    AnchorBuildStatus, FragmentActivation, FragmentActivationStatus, FragmentDecodeError,
    decode_fragment_destination,
};
use crate::history::HistoryPosition;
use crate::links::LinkTable;
use crate::render::diff::{DiffRun, ScreenDiff, diff_screen};
use crate::render::output::{OutputOptions, OutputStats, write_diff};
use crate::render::refine::{
    RefineConfig, RefineProgress, RefineStatus, RefinedTarget, SourceRefiner,
};
use crate::render::sink::LayoutSinkStats;
use crate::render::text::{TextCursor, TextError, fit_prefix, graphemes};
use crate::render::viewport::{
    ContentWidthChange, EofCalibrationProgress, TopPosition, ViewportConfig, ViewportError,
    ViewportFrame, ViewportFrameStatus, ViewportState,
};
use crate::screen::{Cell, ContentArea, Screen, ScreenError, StyleRole};
use crate::sidebar::{SidebarModel, SidebarStats};

pub const FIRST_FRAME_CLEAR: &[u8] = b"\x1b[2J\x1b[H";
pub const BEGIN_SYNCHRONIZED_UPDATE: &[u8] = b"\x1b[?2026h";
pub const END_SYNCHRONIZED_UPDATE: &[u8] = b"\x1b[?2026l";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameGeometry {
    pub rows: u16,
    pub cols: u16,
    pub content_width: u16,
}

impl FrameGeometry {
    fn validate(self) -> Result<(), FrameError> {
        if self.rows == 0 || self.cols == 0 || self.content_width == 0 {
            return Err(FrameError::InvalidGeometry);
        }
        if self.content_width > self.cols {
            return Err(FrameError::InvalidGeometry);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameResize {
    pub previous_rows: u16,
    pub previous_cols: u16,
    pub rows: u16,
    pub cols: u16,
    pub previous_content_width: u16,
    pub content_width: u16,
    pub content_x: u16,
    pub sidebar_width: u16,
    pub sidebar_visible: bool,
    pub height_changed: bool,
    pub width_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarLayout {
    pub visible: bool,
    pub sidebar_width: u16,
    pub content_x: u16,
    pub content_width: u16,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameCommit {
    pub status: Option<ViewportFrameStatus>,
    pub committed: bool,
    pub rows: u16,
    pub bytes: usize,
    pub output: OutputStats,
    pub cleared: bool,
    pub first_frame: bool,
    pub full_repaint: bool,
    pub metrics: FrameMetrics,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameMetrics {
    pub layout_budget: WorkLimit,
    pub planned_tiles: usize,
    pub visible_tiles: usize,
    pub recovery_tiles: usize,
    pub diff_rows: usize,
    pub output_bytes: usize,
    pub body_bytes: usize,
    pub clipboard_bytes: usize,
    pub layout: LayoutSinkStats,
    pub frame_elapsed: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryRestoreStatus {
    Ready {
        moved: bool,
        position: HistoryPosition,
        refine: RefineStatus,
    },
    BudgetExhausted,
    NotFound,
    Failed,
}

impl HistoryRestoreStatus {
    #[must_use]
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    #[must_use]
    pub const fn moved(self) -> bool {
        matches!(self, Self::Ready { moved: true, .. })
    }
}

#[derive(Debug)]
pub enum FrameError {
    InvalidGeometry,
    InputFinished,
    RowOverflow,
    BodyOverflow {
        row: u16,
        column: u16,
        width: u8,
        content_width: u16,
    },
    Viewport(ViewportError),
    Screen(ScreenError),
    Sidebar(LayoutError),
    Text(TextError),
    Output(io::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry => formatter.write_str("invalid terminal frame geometry"),
            Self::InputFinished => formatter.write_str("input is already finished"),
            Self::RowOverflow => formatter.write_str("viewport frame exceeds terminal rows"),
            Self::BodyOverflow {
                row,
                column,
                width,
                content_width,
            } => write!(
                formatter,
                "frame cell at row {row} column {column} width {width} exceeds content width {content_width}"
            ),
            Self::Viewport(error) => error.fmt(formatter),
            Self::Screen(error) => error.fmt(formatter),
            Self::Sidebar(error) => error.fmt(formatter),
            Self::Text(error) => error.fmt(formatter),
            Self::Output(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<ViewportError> for FrameError {
    fn from(error: ViewportError) -> Self {
        Self::Viewport(error)
    }
}

impl From<ScreenError> for FrameError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

impl From<LayoutError> for FrameError {
    fn from(error: LayoutError) -> Self {
        Self::Sidebar(error)
    }
}

impl From<TextError> for FrameError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Output(error)
    }
}

pub struct FrameRuntime {
    document: Utf8Document,
    viewport: ViewportState,
    navigator: Navigator,
    refiner: Option<SourceRefiner>,
    pending_jump: Option<usize>,
    pending_search_target: Option<(usize, bool)>,
    reconcile_scrolled_source: bool,
    presenter: FramePresenter,
    geometry: FrameGeometry,
    link_mode: LinkMode,
    content_x: u16,
    sidebar_width: u16,
    sidebar_visible: bool,
    sidebar_selection: Option<HeadingId>,
    sidebar: Option<SidebarState>,
    links: Option<LinkTable>,
    link_focus: Option<LinkId>,
    pending_activation: Option<(
        LinkId,
        u64,
        mdtui3::LinkActivity,
        crate::render::objects::LinkLease,
    )>,
    anchors: Option<HeadingAnchors>,
    anchor_lease: Option<crate::render::objects::LinkLease>,
    notice: Option<&'static str>,
    reading: crate::reading::ReadingTools,
    reading_first: bool,
    reading_edge_scroll: bool,
    cached_body: Option<ViewportFrame>,
    body_dirty: bool,
}

struct SidebarState {
    model: SidebarModel,
    current: Option<HeadingId>,
    selection: Option<HeadingId>,
}

#[derive(Clone, Copy)]
struct FramePaintLayout<'a> {
    content_x: u16,
    content_width: u16,
    sidebar_width: u16,
    sidebar: Option<&'a SidebarState>,
    focused_link: Option<LinkId>,
    notice: Option<&'a str>,
    following: bool,
    table_notice: bool,
    selection: Option<mdtui3::Span>,
    cursor: Option<mdtui3::Span>,
    search: Option<mdtui3::Span>,
}

#[derive(Clone, Copy)]
struct SidebarWindow {
    current: Option<HeadingId>,
    selection: HeadingId,
    first_visible: usize,
    rows: usize,
    width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefineJump {
    Ready(bool),
    Pending,
    NotFound,
    Failed,
}

enum RefineTarget {
    Ready(RefinedTarget),
    Empty,
    Pending,
    NotFound,
    Failed,
}

impl FrameRuntime {
    pub fn new(geometry: FrameGeometry, link_mode: LinkMode) -> Result<Self, FrameError> {
        Self::from_utf8_document(Utf8Document::new(), geometry, link_mode)
    }

    pub fn from_finished_source(
        source: &str,
        geometry: FrameGeometry,
        link_mode: LinkMode,
    ) -> Result<Self, FrameError> {
        let mut document = Utf8Document::new();
        if !source.is_empty() {
            document
                .append(source.as_bytes())
                .map_err(|_| FrameError::InputFinished)?;
        }
        document.finish();
        Self::from_utf8_document(document, geometry, link_mode)
    }

    pub fn from_utf8_document(
        document: Utf8Document,
        geometry: FrameGeometry,
        link_mode: LinkMode,
    ) -> Result<Self, FrameError> {
        geometry.validate()?;
        let viewport = ViewportState::new(
            document.document(),
            ViewportConfig {
                width: geometry.content_width,
                rows: geometry.rows,
                ..ViewportConfig::default()
            },
        )?;
        let presenter = FramePresenter::new(
            geometry.rows,
            geometry.cols,
            OutputOptions {
                link_mode,
                allow_osc8: true,
            },
        )?;
        Ok(Self {
            document,
            viewport,
            navigator: Navigator::new(),
            refiner: None,
            pending_jump: None,
            pending_search_target: None,
            reconcile_scrolled_source: true,
            presenter,
            geometry,
            link_mode,
            content_x: 0,
            sidebar_width: 0,
            sidebar_visible: false,
            sidebar_selection: None,
            sidebar: None,
            links: None,
            link_focus: None,
            pending_activation: None,
            anchors: None,
            anchor_lease: None,
            notice: None,
            reading: crate::reading::ReadingTools::default(),
            reading_first: true,
            reading_edge_scroll: false,
            cached_body: None,
            body_dirty: true,
        })
    }

    pub fn reading(&self) -> &crate::reading::ReadingTools {
        &self.reading
    }

    /// Handle reading modes before the application's ordinary key bindings.
    pub fn reading_key(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
        body: bool,
    ) -> Result<bool, FrameError> {
        use crate::reading::{CopyKind, CursorMove};
        use crossterm::event::{KeyCode, KeyModifiers};
        let links = self.viewport.shared_links();
        if matches!(code, KeyCode::Esc | KeyCode::Char('/'))
            && self.pending_search_target.take().is_some()
        {
            self.pending_jump = None;
            self.refiner = None;
        }
        if self
            .reading
            .input_key(code, modifiers, self.document.document(), &links)
        {
            return Ok(true);
        }
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            return Ok(false);
        }
        if modifiers.contains(KeyModifiers::ALT) {
            return Ok(false);
        }
        if code == KeyCode::Esc {
            self.reading.cancel(&links);
            return Ok(true);
        }
        if code == KeyCode::Char('/') && !modifiers.contains(KeyModifiers::CONTROL) {
            self.reading.start_input();
            return Ok(true);
        }
        if matches!(code, KeyCode::Char('n' | 'N')) && !modifiers.contains(KeyModifiers::CONTROL) {
            self.reading
                .navigate(self.document.document(), &links, code == KeyCode::Char('N'));
            return Ok(true);
        }
        if !body {
            return Ok(false);
        }
        let area = ContentArea::new(self.content_x, self.geometry.content_width);
        if code == KeyCode::Char('v') && !modifiers.contains(KeyModifiers::CONTROL) {
            self.reading.selection.toggle(self.presenter.screen(), area);
            self.pause_following();
            return Ok(true);
        }
        if !self.reading.selection.active() {
            return Ok(false);
        }
        if matches!(
            code,
            KeyCode::End | KeyCode::Home | KeyCode::Tab | KeyCode::BackTab
        ) {
            self.reading.selection.suspend();
            return Ok(false);
        }
        match code {
            KeyCode::Char(' ') => self.reading.selection.toggle_extension(),
            KeyCode::Char('y' | 'Y') => self.reading.start_copy(
                self.document.document(),
                if code == KeyCode::Char('Y') {
                    CopyKind::Source
                } else {
                    CopyKind::Visible
                },
                self.geometry.content_width,
            ),
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                if !modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let direction = match code {
                    KeyCode::Left => CursorMove::Left,
                    KeyCode::Right => CursorMove::Right,
                    KeyCode::Up => CursorMove::Up,
                    _ => CursorMove::Down,
                };
                self.pause_following();
                if let Some(edge) = self.reading.selection.move_cursor(
                    self.presenter.screen(),
                    area,
                    direction,
                    modifiers.contains(KeyModifiers::SHIFT),
                ) {
                    let moved = if matches!(edge, CursorMove::Left | CursorMove::Up) {
                        self.scroll_up(1)?
                    } else {
                        self.scroll_down(1)?
                    };
                    if !moved {
                        self.reading.selection.cancel_motion();
                    }
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub fn advance_reading<W: Write + ?Sized>(&mut self, writer: &mut W, budget: &mut Budget) {
        let links = self.viewport.shared_links();
        if let Some(source) = self.reading.advance(
            self.document.document(),
            &links,
            budget,
            writer,
            self.viewport.table_widths_mut(),
        ) {
            self.viewport.pause_following();
            self.pending_jump = Some(source);
            self.pending_search_target = Some((source, false));
            self.body_dirty = true;
        }
    }

    #[must_use]
    pub fn document(&self) -> &Document {
        self.document.document()
    }

    pub fn set_table_policy(
        &mut self,
        policy: super::table_widths::TableWidthPolicy,
    ) -> Result<bool, FrameError> {
        let changed = self
            .viewport
            .set_table_policy(self.document.document(), policy)?;
        if changed {
            self.body_dirty = true;
            self.reading.view_changed();
        }
        Ok(changed)
    }

    pub fn pan_table(&mut self, delta: isize) -> Result<bool, FrameError> {
        let Some(status) = self.viewport.shown_tables.first().copied() else {
            return Ok(false);
        };
        let first = self
            .viewport
            .table_widths()
            .first_column(status.table_id)
            .saturating_add_signed(delta);
        let changed = self.viewport.set_table_first_column(
            self.document.document(),
            status.table_id,
            first,
        )?;
        if changed {
            self.body_dirty = true;
            self.reading.view_changed();
            self.reconcile_scrolled_source = true;
        }
        Ok(changed)
    }

    pub fn document_and_viewport_mut(&mut self) -> (&Document, &mut ViewportState) {
        (self.document.document(), &mut self.viewport)
    }

    #[must_use]
    pub const fn geometry(&self) -> FrameGeometry {
        self.geometry
    }

    #[must_use]
    pub const fn link_mode(&self) -> LinkMode {
        self.link_mode
    }

    #[must_use]
    pub const fn content_x(&self) -> u16 {
        self.content_x
    }

    #[must_use]
    pub const fn sidebar_width(&self) -> u16 {
        self.sidebar_width
    }

    #[must_use]
    pub const fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    #[must_use]
    pub fn sidebar_pending(&self) -> bool {
        self.sidebar
            .as_ref()
            .is_some_and(|sidebar| !sidebar.model.is_complete())
    }

    #[must_use]
    pub fn sidebar_current_heading(&self) -> Option<HeadingId> {
        self.sidebar.as_ref().and_then(|sidebar| sidebar.current)
    }

    #[must_use]
    pub fn body_current_heading(&self) -> Option<HeadingId> {
        let document = self.document.document();
        document
            .headings()
            .current(self.viewport.top_source(document).unwrap_or(0))
    }

    #[must_use]
    pub fn sidebar_stats(&self) -> Option<SidebarStats> {
        self.sidebar.as_ref().map(|sidebar| sidebar.model.stats())
    }

    #[must_use]
    pub fn sidebar_window_start(&self) -> Option<usize> {
        self.sidebar
            .as_ref()
            .map(|sidebar| sidebar.model.first_visible())
    }

    #[must_use]
    pub const fn sidebar_selection(&self) -> Option<HeadingId> {
        self.sidebar_selection
    }

    #[must_use]
    pub fn link_table(&self) -> Option<&LinkTable> {
        self.links.as_ref()
    }

    #[must_use]
    pub const fn link_focus(&self) -> Option<LinkId> {
        self.link_focus
    }

    #[must_use]
    pub fn anchor_index_started(&self) -> bool {
        self.anchors.is_some()
    }

    #[must_use]
    pub fn anchor_indexed_headings(&self) -> Option<usize> {
        self.anchors.as_ref().map(HeadingAnchors::indexed_headings)
    }

    #[must_use]
    pub fn anchor_index_complete(&self) -> bool {
        self.anchors.as_ref().is_some_and(|anchors| {
            anchors.revision() == self.document.document().revision() && anchors.is_complete()
        })
    }

    #[must_use]
    pub const fn notice(&self) -> Option<&'static str> {
        self.notice
    }

    pub fn set_notice(&mut self, notice: Option<&'static str>) -> bool {
        if self.notice == notice {
            return false;
        }
        self.notice = notice;
        true
    }

    #[must_use]
    pub fn navigator(&self) -> &Navigator {
        &self.navigator
    }

    pub fn has_pending_jump(&self) -> bool {
        self.pending_jump.is_some() || self.pending_search_target.is_some()
    }

    pub fn viewport(&self) -> &ViewportState {
        &self.viewport
    }

    pub fn viewport_mut(&mut self) -> &mut ViewportState {
        &mut self.viewport
    }

    #[must_use]
    pub fn screen(&self) -> &Screen {
        self.presenter.screen()
    }

    #[must_use]
    pub fn top_source(&self) -> Option<usize> {
        self.viewport.top_source(self.document.document())
    }

    #[must_use]
    pub fn navigation_position(&self) -> Option<HistoryPosition> {
        Some(HistoryPosition {
            source: self.top_source()?,
            row_offset: self.viewport.top_anchor()?.offset,
        })
    }

    pub fn restore_navigation_position(
        &mut self,
        position: HistoryPosition,
        budget: &mut Budget,
    ) -> Result<HistoryRestoreStatus, FrameError> {
        self.body_dirty = true;
        self.cancel_end()?;
        let target = match self.refine_target(position.source, budget)? {
            RefineTarget::Ready(target) => target,
            RefineTarget::Empty => {
                return Ok(HistoryRestoreStatus::Ready {
                    moved: false,
                    position: HistoryPosition {
                        source: position.source,
                        row_offset: 0,
                    },
                    refine: RefineStatus::Approximate,
                });
            }
            RefineTarget::Pending => return Ok(HistoryRestoreStatus::BudgetExhausted),
            RefineTarget::NotFound => return Ok(HistoryRestoreStatus::NotFound),
            RefineTarget::Failed => return Ok(HistoryRestoreStatus::Failed),
        };
        let height = self
            .viewport
            .heights()
            .height(target.anchor.tile)
            .unwrap_or(0);
        let anchor = Anchor {
            tile: target.anchor.tile,
            offset: position.row_offset.min(height.saturating_sub(1)),
        };
        let moved = self.viewport.set_top_from_refinement(
            self.document.document(),
            anchor,
            target.line_start,
            target.line_start,
            target.status != RefineStatus::Approximate,
        )?;
        self.navigator.navigate(
            self.document.document(),
            self.viewport.heights(),
            NavAction::Source(position.source),
            u64::from(self.geometry.rows),
        );
        if let Some(row) = self.viewport.heights().row_of(anchor) {
            self.navigator
                .reconcile(self.viewport.heights(), row, u64::from(self.geometry.rows));
        }
        self.reconcile_scrolled_source = false;
        if moved {
            self.links = None;
        }
        let restored = self.navigation_position().unwrap_or(HistoryPosition {
            source: target.line_start,
            row_offset: anchor.offset,
        });
        Ok(HistoryRestoreStatus::Ready {
            moved,
            position: restored,
            refine: target.status,
        })
    }

    pub fn seek_end(&mut self, follow: bool) -> Result<bool, FrameError> {
        self.pending_search_target = None;
        self.body_dirty = true;
        self.pending_jump = None;
        self.refiner = None;
        self.reconcile_scrolled_source = true;
        let rows = if follow {
            self.geometry.rows.saturating_sub(1).max(1)
        } else {
            self.geometry.rows
        };
        if self.viewport.viewport_rows() != rows {
            self.viewport.set_viewport_rows(rows)?;
        }
        let changed = self.viewport.seek_end(self.document.document(), follow)?;
        if changed {
            self.links = None;
        }
        Ok(changed)
    }

    pub fn pause_following(&mut self) -> bool {
        self.viewport.pause_following()
    }

    pub fn cancel_end(&mut self) -> Result<bool, FrameError> {
        let changed = self.viewport.cancel_end();
        let rows = if self.reading.status_visible() {
            self.geometry.rows.saturating_sub(1).max(1)
        } else {
            self.geometry.rows
        };
        if self.viewport.viewport_rows() != rows {
            self.viewport.set_viewport_rows(rows)?;
        }
        Ok(changed)
    }

    pub fn scroll_down(&mut self, rows: u64) -> Result<bool, FrameError> {
        self.pending_search_target = None;
        self.body_dirty = true;
        self.cancel_end()?;
        self.pending_jump = None;
        self.refiner = None;
        let moved = self.viewport.scroll_down(self.document.document(), rows)?;
        if moved {
            self.reconcile_scrolled_source = true;
            self.links = None;
        }
        Ok(moved)
    }

    pub fn scroll_up(&mut self, rows: u64) -> Result<bool, FrameError> {
        self.pending_search_target = None;
        self.body_dirty = true;
        self.cancel_end()?;
        self.pending_jump = None;
        self.refiner = None;
        let moved = self.viewport.scroll_up(self.document.document(), rows)?;
        if moved {
            self.reconcile_scrolled_source = true;
            self.links = None;
        }
        Ok(moved)
    }

    pub fn append_bytes(&mut self, bytes: &[u8]) -> Result<Option<Update>, FrameError> {
        let update = self
            .document
            .append(bytes)
            .map_err(|_| FrameError::InputFinished)?;
        Ok((update.revision != update.base_revision).then_some(update))
    }

    pub fn apply_update(&mut self, update: &Update) -> Result<(), FrameError> {
        if self.pending_search_target.take().is_some() {
            self.pending_jump = None;
            self.refiner = None;
        }
        self.body_dirty = true;
        self.reading.changed(self.document.document().revision());
        let document = self.document.document();
        let applied = self.viewport.apply_update(document, update)?;
        if applied.previous_revision != applied.revision {
            self.links = None;
            self.anchors = None;
            self.anchor_lease = None;
        }
        self.apply_sidebar_update(update)?;
        Ok(())
    }

    pub fn finish_input(&mut self) -> Result<Option<Update>, FrameError> {
        let update = self.document.finish();
        Ok((update.revision != update.base_revision).then_some(update))
    }

    pub fn advance_eof_calibration(
        &mut self,
        budget: &mut Budget,
    ) -> Result<EofCalibrationProgress, FrameError> {
        Ok(self
            .viewport
            .advance_eof_calibration(self.document.document(), budget)?)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<FrameResize, FrameError> {
        self.body_dirty = true;
        if rows == 0 || cols == 0 {
            return Err(FrameError::InvalidGeometry);
        }
        let reading_source = (self.reading.selection.active() || !self.reading.query().is_empty())
            .then(|| self.top_source())
            .flatten();
        let previous_rows = self.geometry.rows;
        let previous_cols = self.geometry.cols;
        let previous_content_width = self.geometry.content_width;
        let previous_content_x = self.content_x;
        let previous_sidebar_visible = self.sidebar_visible;
        let (sidebar_visible, sidebar_width, content_width) =
            sidebar_layout_for(cols, self.sidebar_visible);
        let width_changed = content_width != previous_content_width;
        let x_changed = sidebar_width != previous_content_x;
        let height_changed = rows != previous_rows;
        let terminal_changed = rows != previous_rows || cols != previous_cols;

        if width_changed {
            self.reading.width_changed();
            self.viewport
                .set_content_width(self.document.document(), content_width)?;
        }
        if height_changed {
            self.viewport
                .set_viewport_rows(if self.viewport.following() {
                    rows.saturating_sub(1).max(1)
                } else {
                    rows
                })?;
        }
        if terminal_changed || width_changed {
            self.presenter.resize(rows, cols)?;
        }
        self.geometry = FrameGeometry {
            rows,
            cols,
            content_width,
        };
        self.content_x = sidebar_width;
        self.sidebar_width = sidebar_width;
        self.sidebar_visible = sidebar_visible;
        if (width_changed || height_changed)
            && !self.viewport.following()
            && let Some(source) = reading_source
        {
            self.pending_jump = Some(source);
            self.refiner = None;
        }
        if width_changed
            || x_changed
            || height_changed
            || previous_sidebar_visible != sidebar_visible
        {
            self.sidebar = None;
            self.links = None;
        }
        Ok(FrameResize {
            previous_rows,
            previous_cols,
            rows,
            cols,
            previous_content_width,
            content_width,
            content_x: self.content_x,
            sidebar_width,
            sidebar_visible,
            height_changed,
            width_changed,
        })
    }

    pub fn set_content_width(&mut self, width: u16) -> Result<ContentWidthChange, FrameError> {
        if width != self.geometry.content_width {
            self.reading.width_changed();
        }
        if width == 0 {
            return Err(FrameError::InvalidGeometry);
        }
        self.set_body_layout(self.content_x, width)
    }

    pub fn set_sidebar_visible(&mut self, visible: bool) -> Result<SidebarLayout, FrameError> {
        let (effective, sidebar_width, width) = sidebar_layout_for(self.geometry.cols, visible);
        let previous_visible = self.sidebar_visible;
        let previous_x = self.content_x;
        let previous_width = self.geometry.content_width;
        let change = self.set_body_layout(sidebar_width, width)?;
        let changed = change.changed
            || previous_visible != effective
            || previous_x != sidebar_width
            || previous_width != width;
        self.sidebar_visible = effective;
        self.sidebar_width = sidebar_width;
        if !effective || changed {
            self.sidebar = None;
        }
        if changed && !change.changed && previous_x == sidebar_width && previous_width == width {
            self.presenter.reset_baseline()?;
        }
        Ok(SidebarLayout {
            visible: effective,
            sidebar_width,
            content_x: sidebar_width,
            content_width: width,
            changed,
        })
    }

    fn set_body_layout(
        &mut self,
        content_x: u16,
        width: u16,
    ) -> Result<ContentWidthChange, FrameError> {
        if width == 0
            || content_x
                .checked_add(width)
                .is_none_or(|right| right > self.geometry.cols)
        {
            return Err(FrameError::InvalidGeometry);
        }
        let previous_x = self.content_x;
        if width != self.geometry.content_width {
            self.reading.width_changed();
        }
        let change = self
            .viewport
            .set_content_width(self.document.document(), width)?;
        let geometry_changed = change.changed || previous_x != content_x;
        self.geometry.content_width = width;
        self.content_x = content_x;
        if geometry_changed {
            self.body_dirty = true;
            self.presenter.reset_baseline()?;
            self.sidebar = None;
            self.links = None;
        }
        Ok(change)
    }

    pub fn frame_to<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        budget: &mut Budget,
    ) -> Result<FrameCommit, FrameError> {
        let started = Instant::now();
        let rows = if self.reading.status_visible() || self.viewport.following() {
            self.geometry.rows.saturating_sub(1).max(1)
        } else {
            self.geometry.rows
        };
        if self.reading.status_visible() && self.viewport.viewport_rows() != rows {
            self.viewport.set_viewport_rows(rows)?;
        }
        if self.reading_edge_scroll
            && budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            })
        {
            self.reading_edge_scroll = false;
            if let Some(direction) = self.reading.selection.pending_motion() {
                let moved = if matches!(
                    direction,
                    crate::reading::CursorMove::Left | crate::reading::CursorMove::Up
                ) {
                    self.scroll_up(1)?
                } else {
                    self.scroll_down(1)?
                };
                if !moved {
                    self.reading.selection.cancel_motion();
                }
            }
        }
        let clipboard_before = self.reading.clipboard_bytes();
        let reading_first = self.reading_first;
        self.reading_first = !reading_first;
        if reading_first {
            self.advance_reading(writer, budget);
        }
        let clipboard_bytes = self.reading.clipboard_bytes() - clipboard_before;
        if let Some((source, false)) = self.pending_search_target {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            }) {
                return Ok(FrameCommit {
                    status: Some(ViewportFrameStatus::BudgetExhausted),
                    bytes: clipboard_bytes,
                    metrics: FrameMetrics {
                        layout_budget: budget.used(),
                        clipboard_bytes,
                        output_bytes: clipboard_bytes,
                        ..FrameMetrics::default()
                    },
                    ..FrameCommit::default()
                });
            }
            let document = self.document.document();
            if let Some(id) = document.table_at(source)
                && let Some(table) = document.table(id)
                && let Some(row) = table.row_at(source).and_then(|row| table.row(row))
            {
                let column = row.cells.partition_point(|cell| cell.source.end <= source);
                if row
                    .cells
                    .get(column)
                    .is_some_and(|cell| cell.source.start <= source)
                    && self.viewport.set_table_first_column(document, id, column)?
                {
                    self.reading.view_changed();
                }
            }
            self.pending_search_target = Some((source, true));
        }
        if let Some(source) = self.pending_jump
            && matches!(
                self.refine_source_to_top(source, budget)?,
                RefineJump::Pending
            )
        {
            return Ok(FrameCommit {
                status: Some(ViewportFrameStatus::BudgetExhausted),
                bytes: clipboard_bytes,
                metrics: FrameMetrics {
                    layout_budget: budget.used(),
                    clipboard_bytes,
                    output_bytes: clipboard_bytes,
                    frame_elapsed: started.elapsed(),
                    ..FrameMetrics::default()
                },
                ..FrameCommit::default()
            });
        }
        if self.pending_jump.is_none() {
            self.pending_search_target = None;
        }
        let frame = if !self.body_dirty
            && self.reading.status_visible()
            && self
                .cached_body
                .as_ref()
                .is_some_and(|frame| frame.matches(self.document.document()))
        {
            let mut frame = self.cached_body.as_ref().unwrap().clone();
            frame.lines.truncate(usize::from(rows));
            frame.rows = frame.rows.min(rows);
            frame
        } else {
            self.viewport.frame(self.document.document(), budget)?
        };
        if !matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return Ok(FrameCommit {
                status: Some(frame.status),
                rows: frame.rows,
                bytes: clipboard_bytes,
                metrics: FrameMetrics {
                    layout_budget: budget.used(),
                    clipboard_bytes,
                    output_bytes: clipboard_bytes,
                    planned_tiles: frame.stats.planned_tiles,
                    visible_tiles: frame.stats.visible_tiles,
                    recovery_tiles: frame.stats.replay_tiles,
                    layout: frame.stats.layout,
                    frame_elapsed: started.elapsed(),
                    ..FrameMetrics::default()
                },
                ..FrameCommit::default()
            });
        }
        self.prepare_sidebar(budget)?;
        if !reading_first {
            self.advance_reading(writer, budget);
        }
        let clipboard_bytes = self.reading.clipboard_bytes() - clipboard_before;
        let mut commit = self.commit_frame_started(writer, &frame, started)?;
        commit.metrics.layout_budget = budget.used();
        commit.metrics.clipboard_bytes = clipboard_bytes;
        commit.metrics.output_bytes += clipboard_bytes;
        commit.bytes += clipboard_bytes;
        Ok(commit)
    }

    pub fn commit_frame<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        frame: &ViewportFrame,
    ) -> Result<FrameCommit, FrameError> {
        self.commit_frame_started(writer, frame, Instant::now())
    }

    fn commit_frame_started<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        frame: &ViewportFrame,
        started: Instant,
    ) -> Result<FrameCommit, FrameError> {
        self.ensure_sidebar_model()?;
        let table_notice = self
            .viewport
            .shown_tables
            .first()
            .filter(|status| {
                status.first > 0
                    || status.visible < status.total
                    || self.viewport.table_widths().policy()
                        != super::table_widths::TableWidthPolicy::default()
            })
            .map(|status| {
                format!(
                    "{:<width$}",
                    status.message(),
                    width = usize::from(self.geometry.content_width)
                )
            });
        let cursor_visible = self.reading.selection.cursor().is_some_and(|cursor| {
            frame.lines.iter().flat_map(|line| &line.cells).any(|cell| {
                cell.source
                    .is_some_and(|source| crate::reading::overlaps(source, cursor))
            })
        });
        self.reading.selection.set_visible(cursor_visible);
        let match_visible = self.reading.current_match().is_some_and(|hit| {
            frame.lines.iter().flat_map(|line| &line.cells).any(|cell| {
                cell.source
                    .is_some_and(|source| crate::reading::overlaps(source, hit.source))
            })
        });
        self.reading.set_match_visible(match_visible);
        let reading_status = self.reading.status();
        let commit = self.presenter.commit(
            writer,
            self.document.document(),
            frame,
            FramePaintLayout {
                content_x: self.content_x,
                content_width: self.geometry.content_width,
                sidebar_width: self.sidebar_width,
                sidebar: self.sidebar.as_ref(),
                focused_link: self.link_focus,
                notice: reading_status
                    .as_deref()
                    .or(self.notice)
                    .or(table_notice.as_deref()),
                following: self.viewport.following(),
                table_notice: reading_status.is_none()
                    && self.notice.is_none()
                    && table_notice.is_some(),
                selection: self.reading.selection.range(),
                cursor: self
                    .reading
                    .selection
                    .active()
                    .then(|| self.reading.selection.cursor())
                    .flatten(),
                search: self.reading.current_match().map(|hit| hit.source),
            },
            started,
        )?;
        if commit.committed {
            self.cached_body = Some(frame.clone());
            self.body_dirty = false;
            if let Some(anchor) = frame.top.anchor()
                && let Some(row) = self.viewport.heights().row_of(anchor)
            {
                if self.reconcile_scrolled_source {
                    if let Some(source) = self.viewport.top_source(self.document.document()) {
                        self.navigator.reconcile_source(
                            self.viewport.heights(),
                            row,
                            u64::from(self.geometry.rows),
                            source,
                        );
                    }
                } else {
                    self.navigator.reconcile(
                        self.viewport.heights(),
                        row,
                        u64::from(self.geometry.rows),
                    );
                }
            }
            self.rebuild_link_table();
            self.reading.selection.reproject(
                self.presenter.screen(),
                ContentArea::new(self.content_x, self.geometry.content_width),
            );
            self.reading_edge_scroll = self.reading.selection.pending_motion().is_some();
        }
        Ok(commit)
    }

    pub fn commit_empty_frame<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
    ) -> Result<FrameCommit, FrameError> {
        let started = Instant::now();
        let document = self.document.document();
        let frame = ViewportFrame {
            document: document.id(),
            revision: document.revision(),
            lines: Vec::new(),
            top: TopPosition::Empty,
            rows: 0,
            status: ViewportFrameStatus::Ready,
            stats: Default::default(),
        };
        self.commit_frame_started(writer, &frame, started)
    }

    fn prepare_sidebar(&mut self, budget: &mut Budget) -> Result<(), FrameError> {
        self.ensure_sidebar_model()?;
        let document = self.document.document();
        if let Some(sidebar) = self.sidebar.as_mut() {
            sidebar.model.advance(document, budget)?;
        }
        Ok(())
    }

    fn ensure_sidebar_model(&mut self) -> Result<(), FrameError> {
        let document = self.document.document();
        let top_source = self.viewport.top_source(document);
        let Some(window) = sidebar_window_for(
            document,
            top_source,
            self.geometry.rows,
            self.sidebar_width,
            &mut self.sidebar_selection,
        ) else {
            self.sidebar = None;
            return Ok(());
        };
        if let Some(sidebar) = self.sidebar.as_mut()
            && sidebar.model.matches(document)
            && sidebar.model.width() == window.width
        {
            sidebar
                .model
                .reconfigure(document, window.first_visible, window.rows)?;
            sidebar.current = window.current;
            sidebar.selection = Some(window.selection);
            return Ok(());
        }
        let mut model =
            SidebarModel::new(document, window.first_visible, window.rows, window.width);
        model.set_links(self.viewport.shared_links());
        self.sidebar = Some(SidebarState {
            model,
            current: window.current,
            selection: Some(window.selection),
        });
        Ok(())
    }

    fn apply_sidebar_update(&mut self, update: &Update) -> Result<(), FrameError> {
        if !self.sidebar_visible {
            return Ok(());
        }
        let document = self.document.document();
        let top_source = self.viewport.top_source(document);
        let Some(window) = sidebar_window_for(
            document,
            top_source,
            self.geometry.rows,
            self.sidebar_width,
            &mut self.sidebar_selection,
        ) else {
            self.sidebar = None;
            return Ok(());
        };
        if let Some(sidebar) = self.sidebar.as_mut()
            && sidebar.model.document() == document.id()
            && sidebar.model.revision() == update.base_revision
        {
            sidebar
                .model
                .apply_update(document, update, window.first_visible, window.rows)?;
            sidebar.current = window.current;
            sidebar.selection = Some(window.selection);
        }
        Ok(())
    }

    pub fn ensure_sidebar_selection(&mut self) -> Option<HeadingId> {
        let document = self.document.document();
        let top_source = self.viewport.top_source(document);
        sidebar_window_for(
            document,
            top_source,
            self.geometry.rows,
            self.sidebar_width,
            &mut self.sidebar_selection,
        )
        .map(|window| window.selection)
    }

    pub fn sidebar_move_selection(&mut self, delta: isize) -> bool {
        let Some(selection) = self.ensure_sidebar_selection() else {
            return false;
        };
        let total = self.document.document().headings().entries().len();
        if total == 0 {
            return false;
        }
        let index = if delta >= 0 {
            selection
                .0
                .saturating_add(delta.unsigned_abs())
                .min(total - 1)
        } else {
            selection.0.saturating_sub(delta.unsigned_abs())
        };
        self.set_sidebar_selection(index)
    }

    pub fn sidebar_selection_home(&mut self) -> bool {
        let total = self.document.document().headings().entries().len();
        total > 0 && self.set_sidebar_selection(0)
    }

    pub fn sidebar_selection_end(&mut self) -> bool {
        let total = self.document.document().headings().entries().len();
        total > 0 && self.set_sidebar_selection(total - 1)
    }

    fn set_sidebar_selection(&mut self, index: usize) -> bool {
        let total = self.document.document().headings().entries().len();
        if !self.sidebar_visible
            || index >= total
            || self.sidebar_selection == Some(HeadingId(index))
        {
            return false;
        }
        self.sidebar_selection = Some(HeadingId(index));
        true
    }

    pub fn activate_sidebar_selection(&mut self, budget: &mut Budget) -> Result<bool, FrameError> {
        let Some(selection) = self.ensure_sidebar_selection() else {
            return Ok(false);
        };
        let document = self.document.document();
        let Some(heading) = document.headings().get(selection).copied() else {
            return Ok(false);
        };
        Ok(matches!(
            self.refine_source_to_top(heading.source.start, budget)?,
            RefineJump::Ready(true)
        ))
    }

    pub fn enter_link_focus(&mut self, forward: bool) -> Option<LinkId> {
        let table = self.links.as_ref()?;
        let id = if forward { table.first() } else { table.last() }?;
        self.link_focus = Some(id);
        Some(id)
    }

    pub fn cycle_link_focus(&mut self, forward: bool) -> bool {
        let Some(table) = self.links.as_ref() else {
            return false;
        };
        let Some(id) = (if forward {
            table.next(self.link_focus)
        } else {
            table.previous(self.link_focus)
        }) else {
            return false;
        };
        if self.link_focus == Some(id) {
            return false;
        }
        self.link_focus = Some(id);
        true
    }

    pub fn clear_link_focus(&mut self) -> bool {
        self.link_focus.take().is_some()
    }

    pub fn advance_anchor_index(
        &mut self,
        budget: &mut Budget,
    ) -> Result<AnchorBuildStatus, FrameError> {
        let document = self.document.document();
        if !document.is_finished() {
            return Ok(AnchorBuildStatus::PendingDocument);
        }
        if self
            .anchors
            .as_ref()
            .is_some_and(|anchors| anchors.revision() != document.revision())
        {
            self.anchors = None;
            self.anchor_lease = None;
        }
        if self.anchors.is_none() {
            self.anchors = Some(HeadingAnchors::new(document));
        }
        let Some(anchors) = self.anchors.as_mut() else {
            return Ok(AnchorBuildStatus::BudgetExhausted);
        };
        if anchors.is_complete() {
            return Ok(AnchorBuildStatus::Complete);
        }
        let links = self.viewport.shared_links();
        if !links.prepare(document, budget)? {
            return Ok(AnchorBuildStatus::BudgetExhausted);
        }
        if self.anchor_lease.is_none() {
            self.anchor_lease = Some(links.hold());
        }
        let completion = links.with_resolver(|resolver| {
            anchors.swap_link_resolver(resolver)?;
            let result = anchors.advance(document, budget);
            anchors.swap_link_resolver(resolver)?;
            result
        })?;
        match completion {
            Completion::EndOfDocument => {
                self.anchor_lease = None;
                Ok(AnchorBuildStatus::Complete)
            }
            _ => Ok(AnchorBuildStatus::BudgetExhausted),
        }
    }

    pub fn activate_focused_link(
        &mut self,
        budget: &mut Budget,
    ) -> Result<FragmentActivation, FrameError> {
        let Some(id) = self.link_focus else {
            return Ok(FragmentActivation::status(FragmentActivationStatus::NoLink));
        };
        let document = self.document.document();
        let links = self.viewport.shared_links();
        if !links.prepare(document, budget)? {
            return Ok(FragmentActivation::status(
                FragmentActivationStatus::BudgetExhausted,
            ));
        }
        if self
            .pending_activation
            .as_ref()
            .is_none_or(|(pending, revision, _, _)| {
                *pending != id || *revision != document.revision()
            })
        {
            let Some(activity) = document.link_activity(id) else {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::Invalid,
                ));
            };
            self.pending_activation = Some((id, document.revision(), activity, links.hold()));
        }
        let disposition = links.with_resolver(|resolver| {
            self.pending_activation
                .as_mut()
                .unwrap()
                .2
                .advance_with(document, resolver, budget)
        })?;
        let Some(disposition) = disposition else {
            return Ok(FragmentActivation::status(
                FragmentActivationStatus::BudgetExhausted,
            ));
        };
        self.pending_activation = None;
        if disposition != mdtui3::LinkDisposition::Active {
            self.link_focus = None;
            return Ok(FragmentActivation::status(
                FragmentActivationStatus::Invalid,
            ));
        }
        let decoded = {
            let document = self.document.document();
            let Some(destination) = document.link_destination(id) else {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::Invalid,
                ));
            };
            if destination.is_empty() {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::Invalid,
                ));
            }
            match decode_fragment_destination(destination) {
                Ok(decoded) => decoded.into_owned(),
                Err(FragmentDecodeError::NotInternal) => {
                    return Ok(FragmentActivation::status(
                        FragmentActivationStatus::External,
                    ));
                }
                Err(FragmentDecodeError::TooLong) => {
                    return Ok(FragmentActivation::status(
                        FragmentActivationStatus::Invalid,
                    ));
                }
            }
        };
        let document = self.document.document();
        if !document.is_finished() {
            return Ok(FragmentActivation::status(
                FragmentActivationStatus::PendingDocument,
            ));
        }
        let Some(anchors) = self.anchors.as_ref() else {
            return Ok(FragmentActivation::status(
                FragmentActivationStatus::PendingIndex,
            ));
        };
        if anchors.revision() != document.revision() || !anchors.is_complete() {
            return Ok(FragmentActivation::status(
                FragmentActivationStatus::PendingIndex,
            ));
        }
        let step = {
            let mut lookup = match anchors.fragment(&decoded) {
                Ok(lookup) => lookup,
                Err(LayoutError::InvalidRange) => {
                    return Ok(FragmentActivation::status(
                        FragmentActivationStatus::Invalid,
                    ));
                }
                Err(error) => return Err(error.into()),
            };
            match lookup.advance(document, budget) {
                Ok(step) => step,
                Err(LayoutError::InvalidRange) => {
                    return Ok(FragmentActivation::status(
                        FragmentActivationStatus::Invalid,
                    ));
                }
                Err(error) => return Err(error.into()),
            }
        };
        let target = match step {
            FragmentStep::Resolved(target) => target,
            FragmentStep::BudgetExhausted => {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::BudgetExhausted,
                ));
            }
            FragmentStep::IndexIncomplete => {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::PendingIndex,
                ));
            }
            FragmentStep::Unresolved => {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::PendingDocument,
                ));
            }
            FragmentStep::NotFound => {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::NotFound,
                ));
            }
        };
        let moved = match self.refine_source_to_top(target.source, budget)? {
            RefineJump::Ready(moved) => moved,
            RefineJump::Pending => {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::BudgetExhausted,
                ));
            }
            RefineJump::NotFound => {
                return Ok(FragmentActivation::status(
                    FragmentActivationStatus::NotFound,
                ));
            }
            RefineJump::Failed => {
                return Ok(FragmentActivation::status(FragmentActivationStatus::Failed));
            }
        };
        Ok(FragmentActivation::moved(target, moved))
    }

    fn refine_source_to_top(
        &mut self,
        source: usize,
        budget: &mut Budget,
    ) -> Result<RefineJump, FrameError> {
        self.body_dirty = true;
        self.cancel_end()?;
        match self.refine_target(source, budget)? {
            RefineTarget::Ready(target) => {
                self.pending_jump = None;
                // Set the exact source first; reconcile refines its row while
                // preserving same-tile heading/fragment identity.
                self.navigator.navigate(
                    self.document.document(),
                    self.viewport.heights(),
                    NavAction::Source(target.source),
                    u64::from(self.geometry.rows),
                );
                self.navigator.reconcile(
                    self.viewport.heights(),
                    target.row,
                    u64::from(self.geometry.rows),
                );
                self.reconcile_scrolled_source = false;
                let moved = self.viewport.set_top_from_refinement(
                    self.document.document(),
                    target.anchor,
                    target.line_start,
                    target.line_start,
                    target.status != RefineStatus::Approximate,
                )?;
                if moved {
                    self.links = None;
                }
                Ok(RefineJump::Ready(moved))
            }
            RefineTarget::Empty => Ok(RefineJump::Ready(false)),
            RefineTarget::Pending => {
                self.pending_jump = Some(source);
                Ok(RefineJump::Pending)
            }
            RefineTarget::NotFound => {
                self.pending_jump = None;
                Ok(RefineJump::NotFound)
            }
            RefineTarget::Failed => {
                self.pending_jump = None;
                Ok(RefineJump::Failed)
            }
        }
    }

    fn refine_target(
        &mut self,
        source: usize,
        budget: &mut Budget,
    ) -> Result<RefineTarget, FrameError> {
        let document = self.document.document();
        if document.tile_count() == 0 {
            return Ok(RefineTarget::Empty);
        }
        let lookup_source = source;
        let viewport_config = self.viewport.config();
        let config = RefineConfig {
            width: self.geometry.content_width,
            max_tiles: viewport_config.max_tiles,
            max_replay_bytes: viewport_config.max_replay_bytes,
            row_slack: viewport_config.row_slack,
        };
        if self.refiner.as_ref().is_some_and(|refiner| {
            !refiner.matches(document)
                || refiner.source() != lookup_source
                || refiner.config() != config
        }) {
            self.refiner = None;
        }
        let shared_links = self.viewport.shared_links();
        let table_navigation = (self.pending_search_target.is_some() && self.refiner.is_none())
            .then(|| {
                (
                    self.viewport.table_widths().policy(),
                    self.viewport.table_widths().column_positions(),
                )
            });
        let progress = {
            let (heights, checkpoints) = self.viewport.layout_parts_mut();
            if self.refiner.is_none() {
                let Ok(mut refiner) =
                    SourceRefiner::new(document, heights, checkpoints, lookup_source, config)
                else {
                    return Ok(RefineTarget::Failed);
                };
                refiner.set_links(shared_links);
                if let Some((policy, positions)) = &table_navigation {
                    refiner
                        .set_table_navigation(
                            document,
                            *policy,
                            positions,
                            usize::from(self.geometry.rows),
                        )
                        .map_err(|error| FrameError::Viewport(ViewportError::FirstFrame(error)))?;
                }
                self.refiner = Some(refiner);
            }
            let Ok(progress) = self.refiner.as_mut().expect("created refiner").advance(
                document,
                heights,
                checkpoints,
                budget,
            ) else {
                self.refiner = None;
                return Ok(RefineTarget::Failed);
            };
            progress
        };
        match progress {
            RefineProgress::Ready(mut target) => {
                self.refiner = None;
                target.source = source;
                Ok(RefineTarget::Ready(target))
            }
            RefineProgress::PendingScan | RefineProgress::BudgetExhausted => {
                Ok(RefineTarget::Pending)
            }
            RefineProgress::NotFound => {
                self.refiner = None;
                Ok(RefineTarget::NotFound)
            }
        }
    }

    fn rebuild_link_table(&mut self) {
        let table = LinkTable::from_screen(
            self.presenter.screen(),
            ContentArea::new(self.content_x, self.geometry.content_width),
        );
        if self
            .link_focus
            .is_some_and(|focused| !table.contains(focused))
        {
            self.link_focus = None;
        }
        self.links = Some(table);
    }
}

struct FramePresenter {
    screen: Screen,
    options: OutputOptions,
    first_clear_sent: bool,
    full_repaint: bool,
}

impl FramePresenter {
    fn new(rows: u16, cols: u16, options: OutputOptions) -> Result<Self, FrameError> {
        let screen = Screen::new(rows, cols, ContentArea::new(0, cols))?;
        Ok(Self {
            screen,
            options,
            first_clear_sent: false,
            full_repaint: false,
        })
    }

    fn screen(&self) -> &Screen {
        &self.screen
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), FrameError> {
        self.screen.resize(rows, cols, ContentArea::new(0, cols))?;
        self.full_repaint = true;
        Ok(())
    }

    fn reset_baseline(&mut self) -> Result<(), FrameError> {
        self.screen.resize(
            self.screen.rows(),
            self.screen.cols(),
            self.screen.content_area(),
        )?;
        self.full_repaint = true;
        Ok(())
    }

    fn commit<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        document: &Document,
        frame: &ViewportFrame,
        layout: FramePaintLayout<'_>,
        started: Instant,
    ) -> Result<FrameCommit, FrameError> {
        self.paint(frame, layout)?;
        self.paint_link_focus(layout)?;
        self.paint_reading(layout)?;
        self.paint_notice(layout)?;
        let cleared = !self.first_clear_sent;
        let full_repaint = self.full_repaint;
        let diff = if full_repaint {
            full_screen_diff(&self.screen)
        } else {
            diff_screen(&self.screen)
        };

        let mut bytes = 0_usize;
        if cleared {
            writer.write_all(FIRST_FRAME_CLEAR)?;
            bytes = bytes.saturating_add(FIRST_FRAME_CLEAR.len());
        }
        let output = if diff_has_output(&diff) {
            if let Err(error) = writer.write_all(BEGIN_SYNCHRONIZED_UPDATE) {
                let _ = writer.write_all(END_SYNCHRONIZED_UPDATE);
                return Err(error.into());
            }
            bytes = bytes.saturating_add(BEGIN_SYNCHRONIZED_UPDATE.len());
            let output = match write_diff(writer, document, &self.screen, &diff, self.options) {
                Ok(output) => output,
                Err(error) => {
                    let _ = writer.write_all(END_SYNCHRONIZED_UPDATE);
                    return Err(error.into());
                }
            };
            writer.write_all(END_SYNCHRONIZED_UPDATE)?;
            bytes = bytes.saturating_add(END_SYNCHRONIZED_UPDATE.len());
            output
        } else {
            write_diff(writer, document, &self.screen, &diff, self.options)?
        };
        bytes = bytes.saturating_add(output.bytes);
        self.screen.swap_buffers();
        let metrics = FrameMetrics {
            layout_budget: frame.stats.layout_budget,
            planned_tiles: frame.stats.planned_tiles,
            visible_tiles: frame.stats.visible_tiles,
            recovery_tiles: frame.stats.replay_tiles,
            diff_rows: diff.rows_changed,
            output_bytes: bytes,
            body_bytes: output.bytes,
            clipboard_bytes: 0,
            layout: frame.stats.layout,
            frame_elapsed: started.elapsed(),
        };
        self.first_clear_sent = true;
        self.full_repaint = false;
        Ok(FrameCommit {
            status: Some(frame.status),
            committed: true,
            rows: frame.rows,
            bytes,
            output,
            cleared,
            first_frame: cleared,
            full_repaint,
            metrics,
        })
    }

    fn paint(
        &mut self,
        frame: &ViewportFrame,
        layout: FramePaintLayout<'_>,
    ) -> Result<(), FrameError> {
        if self.screen.rows() == 0 || self.screen.cols() == 0 {
            return Err(FrameError::InvalidGeometry);
        }
        self.screen.clear_back();
        if layout.sidebar_width > 0 {
            self.paint_sidebar(layout.sidebar_width, layout.sidebar)?;
        }
        for (row, line) in frame.lines.iter().enumerate() {
            let row = u16::try_from(row).map_err(|_| FrameError::RowOverflow)?;
            if row >= self.screen.rows() {
                return Err(FrameError::RowOverflow);
            }
            let mut column = 0_u16;
            for source in &line.cells {
                let width = u16::from(source.width);
                let end = column.checked_add(width).ok_or(FrameError::RowOverflow)?;
                if end > layout.content_width || end > self.screen.cols() {
                    return Err(FrameError::BodyOverflow {
                        row,
                        column,
                        width: source.width,
                        content_width: layout.content_width,
                    });
                }
                self.screen.write_cell(
                    row,
                    layout.content_x + column,
                    Cell::new(
                        source.text.clone(),
                        source.style,
                        source.width,
                        source.link,
                        source.source,
                    )?,
                )?;
                column = end;
            }
            if let Some(source) = line.source {
                self.screen.set_line_source(row, source)?;
            }
        }
        Ok(())
    }

    fn paint_sidebar(
        &mut self,
        sidebar_width: u16,
        sidebar: Option<&SidebarState>,
    ) -> Result<(), FrameError> {
        let border_column = sidebar_width - 1;
        let text_width = usize::from(border_column);
        for row in 0..self.screen.rows() {
            self.screen.write_cell(
                row,
                border_column,
                Cell::new("│", StyleRole::TABLE_BORDER, 1, None, None)?,
            )?;
            let Some(entry) = sidebar.and_then(|sidebar| sidebar.model.entry(usize::from(row)))
            else {
                continue;
            };
            let indent = usize::from(entry.level.saturating_sub(1)).saturating_mul(2);
            if indent >= text_width {
                continue;
            }
            let available = text_width - indent;
            let text = clip_sidebar_text(&entry.text, available);
            let style = match sidebar {
                Some(sidebar)
                    if sidebar.current == Some(entry.id) && sidebar.selection == Some(entry.id) =>
                {
                    StyleRole::SIDEBAR_CURRENT.with(StyleRole::SIDEBAR_SELECTION)
                }
                Some(sidebar) if sidebar.current == Some(entry.id) => StyleRole::SIDEBAR_CURRENT,
                Some(sidebar) if sidebar.selection == Some(entry.id) => {
                    StyleRole::SIDEBAR_SELECTION
                }
                _ => StyleRole::SIDEBAR,
            };
            let mut cursor =
                TextCursor::new(u16::try_from(indent).map_err(|_| FrameError::RowOverflow)?);
            for grapheme in graphemes(&text) {
                cursor.place(
                    &mut self.screen,
                    row,
                    grapheme,
                    style,
                    None,
                    Some(entry.source),
                )?;
            }
        }
        Ok(())
    }

    fn paint_link_focus(&mut self, layout: FramePaintLayout<'_>) -> Result<(), FrameError> {
        let Some(focused) = layout.focused_link else {
            return Ok(());
        };
        let right = layout
            .content_x
            .saturating_add(layout.content_width)
            .min(self.screen.cols());
        for row in 0..self.screen.rows() {
            for column in layout.content_x..right {
                let Some(cell) = self.screen.cell(row, column) else {
                    continue;
                };
                if cell.is_continuation() || cell.link_id() != Some(focused) {
                    continue;
                }
                let style = cell.style().with(StyleRole::LINK_FOCUS);
                self.screen.set_cell_style(row, column, style)?;
            }
        }
        Ok(())
    }

    fn paint_reading(&mut self, layout: FramePaintLayout<'_>) -> Result<(), FrameError> {
        for row in 0..self.screen.rows() {
            for col in layout.content_x
                ..layout
                    .content_x
                    .saturating_add(layout.content_width)
                    .min(self.screen.cols())
            {
                let Some(cell) = self.screen.cell(row, col) else {
                    continue;
                };
                if cell.is_continuation() {
                    continue;
                }
                let Some(source) = cell.source() else {
                    continue;
                };
                let mut style = cell.style();
                if layout
                    .search
                    .is_some_and(|hit| crate::reading::overlaps(source, hit))
                {
                    style = style.with(StyleRole::SEARCH);
                }
                if layout
                    .selection
                    .is_some_and(|range| crate::reading::overlaps(source, range))
                {
                    style = style.with(StyleRole::SELECTION);
                }
                if layout
                    .cursor
                    .is_some_and(|cursor| crate::reading::overlaps(source, cursor))
                {
                    style = style.with(StyleRole::CURSOR);
                }
                self.screen.set_cell_style(row, col, style)?;
            }
        }
        Ok(())
    }

    fn paint_notice(&mut self, layout: FramePaintLayout<'_>) -> Result<(), FrameError> {
        let follow_notice = layout.following.then(|| {
            let mut message = String::from(if layout.table_notice {
                "FOLLOW"
            } else {
                "FOLLOW | Up/PgUp/Home: pause | Ctrl+End: once"
            });
            if let Some(notice) = layout.notice {
                message.push_str(" | ");
                message.push_str(notice);
            }
            message
        });
        let Some(message) = follow_notice.as_deref().or(layout.notice) else {
            return Ok(());
        };
        let Some(row) = self.screen.rows().checked_sub(1) else {
            return Ok(());
        };
        let right = layout
            .content_x
            .saturating_add(layout.content_width)
            .min(self.screen.cols());
        let width = usize::from(right.saturating_sub(layout.content_x));
        if width == 0 {
            return Ok(());
        }
        for col in layout.content_x..right {
            self.screen.write_cell(row, col, Cell::blank())?;
        }
        let fit = fit_prefix(message, width);
        let mut cursor = TextCursor::new(layout.content_x);
        for grapheme in graphemes(&message[..fit.end]) {
            if grapheme.width == 0 {
                continue;
            }
            cursor.place(&mut self.screen, row, grapheme, StyleRole::META, None, None)?;
        }
        Ok(())
    }
}

fn clip_sidebar_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let fit = fit_prefix(text, width);
    if !fit.clipped {
        return text[..fit.end].to_owned();
    }
    let ellipsis_cells = width.saturating_sub(1);
    let fit = fit_prefix(text, ellipsis_cells);
    let mut clipped = text[..fit.end].to_owned();
    clipped.push('…');
    clipped
}

fn diff_has_output(diff: &ScreenDiff) -> bool {
    diff.runs
        .iter()
        .any(|run| run.cells.iter().any(|cell| !cell.is_continuation()))
}

fn sidebar_layout_for(cols: u16, visible: bool) -> (bool, u16, u16) {
    if !visible {
        return (false, 0, cols);
    }
    let Some(width) = crate::app::sidebar_width(cols) else {
        return (false, 0, cols);
    };
    (true, width, cols.saturating_sub(width))
}

fn sidebar_window_for(
    document: &Document,
    top_source: Option<usize>,
    rows: u16,
    sidebar_width: u16,
    selection: &mut Option<HeadingId>,
) -> Option<SidebarWindow> {
    if sidebar_width <= 1 {
        return None;
    }
    let headings = document.headings();
    let total = headings.entries().len();
    if total == 0 {
        *selection = None;
        return None;
    }
    let source = top_source.unwrap_or(0);
    let current = headings.current(source);
    let selected = selection
        .filter(|selected| selected.0 < total)
        .or(current)
        .or_else(|| headings.next(source, 6))
        .unwrap_or(HeadingId(0));
    *selection = Some(selected);
    let rows = usize::from(rows);
    let width = usize::from(sidebar_width - 1);
    if width == 0 {
        return None;
    }
    let max_first = total.saturating_sub(rows);
    let first_visible = selected.0.saturating_sub(rows / 2).min(max_first);
    let visible_current = current
        .filter(|current| (first_visible..first_visible.saturating_add(rows)).contains(&current.0));
    Some(SidebarWindow {
        current: visible_current,
        selection: selected,
        first_visible,
        rows,
        width,
    })
}

fn full_screen_diff(screen: &Screen) -> ScreenDiff {
    let content = screen.content_area();
    if content.width == 0 {
        return ScreenDiff::default();
    }
    let mut diff = ScreenDiff::default();
    for row in 0..screen.rows() {
        let Some(back) = screen.back_row(row) else {
            continue;
        };
        let cells = back[usize::from(content.x)..usize::from(content.right())].to_vec();
        diff.rows_changed += 1;
        diff.cells_changed = diff.cells_changed.saturating_add(cells.len());
        diff.runs.push(DiffRun {
            row,
            start_col: content.x,
            cells,
        });
    }
    diff
}
