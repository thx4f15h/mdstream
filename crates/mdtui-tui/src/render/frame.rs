use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mdtui3::{
    Anchor, Budget, Completion, CurrentFragment, Document, FragmentStep, HeadingId,
    HeadingSlugSession, LayoutError, LinkId, NavAction, Navigator, Update, Utf8Document, WorkLimit,
};

use crate::cli::LinkMode;
use crate::fragment::{AnchorBuildStatus, FragmentActivation, FragmentActivationStatus};
use crate::history::HistoryPosition;
use crate::links::LinkTable;
use crate::render::diff::{DiffRun, ScreenDiff, diff_screen};
use crate::render::output::{OutputOptions, OutputStats};
use crate::render::refine::{
    RefineConfig, RefineProgress, RefineStatus, RefinedTarget, SourceRefiner,
};
use crate::render::sink::LayoutSinkStats;
use crate::render::text::{TextCursor, TextError, fit_prefix, graphemes};
use crate::render::viewport::{
    ContentWidthChange, EofCalibrationProgress, TopPosition, ViewportConfig, ViewportError,
    ViewportFrame, ViewportFrameStatus, ViewportState,
};
use crate::render::wire::{OpenCell, Progress, Transaction};
use crate::screen::{Cell, ContentArea, Screen, ScreenError, StyleRole};
use crate::sidebar::SidebarStats;
use crate::sidebar::directory::{Directory, DirectoryEvent, HEADER_ROWS};

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
    /// Logical back frame published; wire acknowledgement may still be pending.
    pub published: bool,
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
    InitializationPending,
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
            Self::InitializationPending => {
                formatter.write_str("complete budgeted shape initialization before appending")
            }
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

struct PendingFragment {
    id: LinkId,
    revision: u64,
    activity: Option<mdtui3::LinkActivity>,
    active: bool,
    destination: Option<mdtui3::LinkDestinationTask>,
    decoded: String,
    request: Option<CurrentFragment>,
    target: Option<mdtui3::AnchorTarget>,
    refined: Option<RefinedTarget>,
    cancelled: bool,
    waiting: bool,
    _lease: crate::render::objects::LinkLease,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum VisibleObject {
    Link(LinkId),
    Fold(mdtui3::FoldId, mdtui3::Span),
}

/// A committed placeholder run, in terminal coordinates. Its registered range
/// is distinct from the hidden union represented by this one visible object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldHit {
    pub id: mdtui3::FoldId,
    pub owner_tile: mdtui3::TileId,
    pub component_start: usize,
    pub registered: mdtui3::Span,
    pub hidden_envelope: mdtui3::Span,
    pub epoch: u64,
    pub row: u16,
    pub x0: u16,
    pub x1: u16,
}

pub struct FrameRuntime {
    fold_hits: Vec<FoldHit>,
    objects: Vec<VisibleObject>,
    object_focus: Option<VisibleObject>,
    object_version: u64,
    fold_ui: crate::fold_ui::FoldUi,
    fold_jump: Option<(usize, mdtui3::FoldNavigation)>,
    fold_status_rows: bool,
    revealed: Option<(u64, u64, usize)>,
    shape: Option<super::shape::SharedShape>,
    document: Utf8Document,
    viewport: ViewportState,
    navigator: Navigator,
    refiner: Option<SourceRefiner>,
    refiner_focus: Option<super::math::MathFocus>,
    pending_jump: Option<usize>,
    pending_restore: Option<HistoryPosition>,
    restore_result: Option<(HistoryPosition, HistoryRestoreStatus)>,
    pending_search_target: Option<(usize, bool)>,
    pending_sidebar_activate: bool,
    reconcile_scrolled_source: bool,
    presenter: FramePresenter,
    geometry: FrameGeometry,
    link_mode: LinkMode,
    content_x: u16,
    sidebar_width: u16,
    sidebar_visible: bool,
    sidebar: Option<Directory>,
    links: Option<LinkTable>,
    link_focus: Option<LinkId>,
    slugs: HeadingSlugSession,
    anchors_started: bool,
    fragment: Option<PendingFragment>,
    fragment_serial: u64,
    fragment_success: Option<FragmentActivation>,
    anchor_lease: Option<crate::render::objects::LinkLease>,
    notice: Option<&'static str>,
    reading: crate::reading::ReadingTools,
    reading_first: bool,
    reading_edge_scroll: bool,
    cached_body: Option<ViewportFrame>,
    body_dirty: bool,
}

#[derive(Clone, Copy)]
struct FramePaintLayout<'a> {
    content_x: u16,
    content_width: u16,
    sidebar_width: u16,
    sidebar: Option<&'a Directory>,
    focused_link: Option<LinkId>,
    notice: Option<&'a str>,
    following: bool,
    table_notice: bool,
    selection: Option<mdtui3::Span>,
    cursor: Option<mdtui3::Span>,
    search: Option<mdtui3::Span>,
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
    pub fn fold_busy(&self) -> bool {
        self.fold_ui.active()
            || self.fold_jump.is_some()
            || self.pending_jump.is_some()
            || self.pending_restore.is_some()
            || self
                .fragment
                .as_ref()
                .is_some_and(|p| !p.waiting && !p.cancelled)
            || self.pending_sidebar_activate
            || self.viewport.checkpoints().maintenance_in_progress()
    }
    pub fn fold_menu_key(&mut self, code: crossterm::event::KeyCode) -> bool {
        self.fold_ui.key(code)
    }
    /// Diagnostic count of actual body-prefix query starts, excluding retained
    /// metadata checks. This does not enumerate task/checkbox statistics.
    pub fn prefix_query_starts(&self) -> usize {
        self.fold_ui.prefix_query_starts()
    }
    pub fn fold_pending(&self) -> bool {
        self.fold_ui.pending() || self.fold_jump.is_some()
    }
    pub fn open_fold_menu(&mut self, sidebar: bool) {
        self.cancel_fragment();
        self.pending_restore = None;
        self.restore_result = None;
        self.pending_jump = None;
        self.pending_search_target = None;
        self.pending_sidebar_activate = false;
        self.refiner = None;
        self.fold_jump = None;
        self.revealed = None;
        self.reading.cancel_navigation();
        let source = self
            .reading
            .selection
            .cursor()
            .map(|s| s.start)
            .or_else(|| self.top_source())
            .unwrap_or(0);
        let target = if sidebar {
            self.sidebar
                .as_ref()
                .and_then(|s| s.selection())
                .map(mdtui3::FoldId::Heading)
        } else {
            match self.object_focus {
                Some(VisibleObject::Fold(id, _)) => Some(id),
                _ => None,
            }
        };
        self.fold_ui
            .open(source, self.reading.selection.range(), target);
    }
    pub fn invalidate_checkpoints(
        &mut self,
        range: std::ops::Range<usize>,
    ) -> Result<(), mdtui3::LayoutError> {
        self.body_dirty = true;
        self.cached_body = None;
        self.refiner = None;
        self.viewport
            .checkpoints_mut()
            .invalidate(self.document.document(), range)
    }

    pub fn new(geometry: FrameGeometry, link_mode: LinkMode) -> Result<Self, FrameError> {
        Self::from_utf8_document(Utf8Document::new(), geometry, link_mode)
    }

    pub fn from_finished_source(
        source: &str,
        geometry: FrameGeometry,
        link_mode: LinkMode,
    ) -> Result<Self, FrameError> {
        let mut document = Utf8Document::new();
        document.enable_display_shape_effects()?;
        let shape = super::shape::SharedShape::new(document.document())?;
        if !source.is_empty() {
            let update = document
                .append(source.as_bytes())
                .map_err(|_| FrameError::InputFinished)?;
            shape.apply(document.document(), &update)?;
        }
        let update = document.finish();
        shape.apply(document.document(), &update)?;
        Self::from_indexed_document(document, Some(shape), geometry, link_mode)
    }

    pub fn from_utf8_document(
        mut document: Utf8Document,
        geometry: FrameGeometry,
        link_mode: LinkMode,
    ) -> Result<Self, FrameError> {
        if document.document().source().is_empty() && !document.document().is_finished() {
            document.enable_display_shape_effects()?;
            let shape = super::shape::SharedShape::new(document.document())?;
            return Self::from_indexed_document(document, Some(shape), geometry, link_mode);
        }
        document.subscribe_display_shape_effects();
        let shape = super::shape::SharedShape::from_current(document.document());
        let seed = super::shape::ShapeSeed::indexed(document.document());
        let runtime = Self::from_indexed_document(document, Some(shape), geometry, link_mode)?;
        runtime.viewport.shared_links().set_seed(seed);
        Ok(runtime)
    }

    /// Adopt an already parsed document without re-lexing Markdown. The caller
    /// must finish this budgeted seed before supplying another input batch.
    pub fn initialize(&mut self, budget: &mut Budget) -> Result<bool, FrameError> {
        Ok(self
            .viewport
            .shared_links()
            .prepare_shape(self.document.document(), budget)?)
    }

    fn from_indexed_document(
        mut document: Utf8Document,
        shape: Option<super::shape::SharedShape>,
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
        if let Some(shape) = &shape {
            viewport.shared_links().set_shape(shape.clone())?;
        }
        let presenter = FramePresenter::new(
            geometry.rows,
            geometry.cols,
            OutputOptions {
                link_mode,
                allow_osc8: true,
            },
        )?;
        let slugs = document.heading_slug_session();
        Ok(Self {
            shape,
            document,
            viewport,
            navigator: Navigator::new(),
            fold_hits: Vec::new(),
            objects: Vec::new(),
            object_focus: None,
            object_version: 0,
            fold_ui: Default::default(),
            fold_jump: None,
            fold_status_rows: false,
            revealed: None,
            refiner: None,
            refiner_focus: None,
            pending_jump: None,
            pending_restore: None,
            restore_result: None,
            pending_search_target: None,
            pending_sidebar_activate: false,
            reconcile_scrolled_source: true,
            presenter,
            geometry,
            link_mode,
            content_x: 0,
            sidebar_width: 0,
            sidebar_visible: false,
            sidebar: None,
            links: None,
            link_focus: None,
            slugs,
            anchors_started: false,
            fragment: None,
            fragment_serial: 0,
            fragment_success: None,
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
        self.reading.set_width(self.geometry.content_width);
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
        if self.fold_ui.key(code) {
            return Ok(true);
        }
        if code == KeyCode::Char('z') && !modifiers.contains(KeyModifiers::CONTROL) {
            self.open_fold_menu(false);
            return Ok(true);
        }
        if code == KeyCode::Esc {
            self.cancel_fragment();
            self.pending_restore = None;
            self.restore_result = None;
            self.pending_jump = None;
            self.refiner = None;
            self.fold_jump = None;
            let maintaining = self.viewport.checkpoints().maintenance_in_progress();
            if !maintaining && self.viewport.folds().preview().is_some() {
                self.fold_ui.end_preview();
                return Ok(true);
            }
            if maintaining {
                self.fold_ui.end_preview();
            }
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
            KeyCode::Char('y' | 'Y') => {
                let range = self.reading.selection.range();
                let payload = (0..self.geometry.rows)
                    .flat_map(|row| {
                        (self.content_x..self.content_x + self.geometry.content_width)
                            .map(move |column| (row, column))
                    })
                    .find_map(|(row, column)| {
                        [
                            self.presenter.screen().reading_zero_cell(row, column),
                            self.presenter.screen().reading_cell(row, column),
                        ]
                        .into_iter()
                        .flatten()
                        .find_map(|cell| {
                            (cell.source() == range && cell.text().contains_whole_source_atoms())
                                .then(|| cell.text().clone())
                        })
                    });
                self.reading.start_copy_payload(
                    self.document.document(),
                    if code == KeyCode::Char('Y') {
                        CopyKind::Source
                    } else {
                        CopyKind::Visible
                    },
                    self.geometry.content_width,
                    payload,
                );
                if code == KeyCode::Char('y') {
                    self.reading.bind_copy_view(
                        self.viewport.folds().version(),
                        self.viewport.folds().has_collapsed(),
                    );
                }
            }
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
        if self.reading.copy_pending() && self.presenter.wire.is_some() {
            // Clipboard controls wait for a real EGC boundary. The normal
            // frame path continues/ACKs the retained wire transaction; a copy
            // request must never cancel an otherwise unchanged open glyph.
            return;
        }
        if self.viewport.checkpoints().maintenance_in_progress() {
            return;
        }
        let links = self.viewport.shared_links();
        let width = self.presenter.screen().content_area().width;
        self.reading.set_width(width);
        let (owner, tables) = self.viewport.reading_parts_mut();
        if let Some(source) = self.reading.advance_with_view(
            self.document.document(),
            &links,
            budget,
            writer,
            tables,
            Some(owner),
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
        if self.viewport.checkpoints().maintenance_in_progress() {
            return Ok(false);
        }
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
        self.body_dirty = true;
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
        self.sidebar_visible && self.sidebar.as_ref().is_some_and(Directory::pending)
    }

    #[must_use]
    pub fn sidebar_current_heading(&self) -> Option<HeadingId> {
        self.sidebar.as_ref().and_then(Directory::visible_current)
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
        self.sidebar.as_ref().map(Directory::stats)
    }

    #[must_use]
    pub fn sidebar_window_start(&self) -> Option<usize> {
        self.sidebar.as_ref().map(Directory::window_start)
    }

    #[must_use]
    pub fn sidebar_selection(&self) -> Option<HeadingId> {
        self.sidebar.as_ref().and_then(Directory::selection)
    }

    pub fn directory(&self) -> Option<&Directory> {
        self.sidebar.as_ref()
    }
    pub fn directory_mut(&mut self) -> Option<&mut Directory> {
        self.sidebar.as_mut()
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
        self.anchors_started
    }

    #[must_use]
    pub fn anchor_indexed_headings(&self) -> Option<usize> {
        self.anchors_started.then(|| self.slugs.indexed_headings())
    }

    #[must_use]
    pub fn anchor_index_complete(&self) -> bool {
        self.anchors_started && self.slugs.is_current(self.document.document())
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
        self.pending_restore.is_some()
            || self.pending_jump.is_some()
            || self.fragment.as_ref().is_some_and(|p| !p.waiting)
            || self.pending_search_target.is_some()
            || self.pending_sidebar_activate
    }

    pub fn viewport(&self) -> &ViewportState {
        &self.viewport
    }

    pub fn viewport_mut(&mut self) -> &mut ViewportState {
        self.body_dirty = true;
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
        let result = self.restore_position(position, budget)?;
        if result == HistoryRestoreStatus::BudgetExhausted {
            self.pending_restore = Some(position);
            self.restore_result = None;
        } else {
            self.pending_restore = None;
            self.restore_result = Some((position, result));
        }
        Ok(result)
    }

    pub fn history_restore_result(&self) -> Option<(HistoryPosition, HistoryRestoreStatus)> {
        self.restore_result
    }

    fn restore_position(
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
        self.pending_sidebar_activate = false;
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
        self.pending_restore = None;
        self.restore_result = None;
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
        self.pending_sidebar_activate = false;
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
        self.pending_sidebar_activate = false;
        self.refiner = None;
        let moved = self.viewport.scroll_up(self.document.document(), rows)?;
        if moved {
            self.reconcile_scrolled_source = true;
            self.links = None;
        }
        Ok(moved)
    }

    pub fn append_bytes(&mut self, bytes: &[u8]) -> Result<Option<Arc<Update>>, FrameError> {
        if !self.viewport.shared_links().shape_ready() {
            return Err(FrameError::InitializationPending);
        }
        let update = self
            .document
            .append(bytes)
            .map_err(|_| FrameError::InputFinished)?;
        if let Some(shape) = &self.shape {
            shape.apply(self.document.document(), &update)?;
        }
        if update.revision == update.base_revision {
            return Ok(None);
        }
        let update = Arc::new(update);
        self.slugs
            .observe(self.document.document(), update.clone())?;
        Ok(Some(update))
    }

    pub fn apply_update(&mut self, update: &Update) -> Result<(), FrameError> {
        if self.pending_search_target.take().is_some() {
            self.pending_jump = None;
            self.refiner = None;
        }
        if let Some(p) = &mut self.fragment {
            p.waiting = false;
        }
        self.body_dirty = true;
        self.fold_jump = None;
        self.revealed = None;
        self.refiner = None;
        self.reading.changed(self.document.document().revision());
        let document = self.document.document();
        let applied = self.viewport.apply_update(document, update)?;
        self.fold_ui
            .on_update(document, self.viewport.folds(), update)
            .map_err(ViewportError::Layout)?;
        if applied.previous_revision != applied.revision {
            self.links = None;
        }
        self.apply_sidebar_update(update)?;
        Ok(())
    }

    pub fn finish_input(&mut self) -> Result<Option<Arc<Update>>, FrameError> {
        if !self.viewport.shared_links().shape_ready() {
            return Err(FrameError::InitializationPending);
        }
        let update = self.document.finish();
        if let Some(shape) = &self.shape {
            shape.apply(self.document.document(), &update)?;
        }
        if update.revision == update.base_revision {
            return Ok(None);
        }
        let update = Arc::new(update);
        self.slugs
            .observe(self.document.document(), update.clone())?;
        Ok(Some(update))
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
            self.fold_ui.resize();
            self.fold_jump = None;
            self.revealed = None;
            self.refiner = None;
            if let Some(fragment) = &mut self.fragment {
                fragment.refined = None;
            }
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
        self.reading.set_width(content_width);
        self.content_x = sidebar_width;
        self.sidebar_width = sidebar_width;
        self.sidebar_visible = sidebar_visible;
        if previous_sidebar_visible
            && !sidebar_visible
            && let Some(sidebar) = self.sidebar.as_mut()
        {
            sidebar.suspend();
        }
        if width_changed
            || x_changed
            || height_changed
            || previous_sidebar_visible != sidebar_visible
        {
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
            self.fold_ui.resize();
            if let Some(fragment) = &mut self.fragment {
                fragment.refined = None;
            }
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
        if !effective {
            self.pending_sidebar_activate = false;
            if let Some(sidebar) = self.sidebar.as_mut() {
                sidebar.suspend();
            }
        }
        self.sidebar_width = sidebar_width;
        self.ensure_sidebar_model()?;
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
            self.fold_ui.resize();
            if let Some(fragment) = &mut self.fragment {
                fragment.refined = None;
            }
            self.reading.width_changed();
        }
        let change = self
            .viewport
            .set_content_width(self.document.document(), width)?;
        let geometry_changed = change.changed || previous_x != content_x;
        self.geometry.content_width = width;
        self.reading.set_width(width);
        self.content_x = content_x;
        if geometry_changed {
            self.body_dirty = true;
            self.presenter.reset_baseline()?;
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
        if self.viewport.checkpoints().maintenance_in_progress() {
            let progress = self
                .viewport
                .maintain_checkpoints(self.document.document(), budget)?;
            if progress.completion == Completion::BudgetExhausted {
                return Ok(FrameCommit {
                    status: Some(ViewportFrameStatus::BudgetExhausted),
                    metrics: FrameMetrics {
                        layout_budget: budget.used(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            }
        }
        if self.fold_ui.pending() && !self.viewport.checkpoints().maintenance_in_progress() {
            let links = self.viewport.shared_links();
            let mut limit = budget.remaining();
            limit.queries = limit.queries.div_ceil(2);
            limit.objects = limit.objects.div_ceil(2);
            let mut work = Budget::new(limit);
            self.fold_ui.advance(
                self.document.document(),
                self.viewport.folds_mut(),
                &links,
                self.geometry.content_width,
                &mut work,
            );
            assert!(budget.charge(work.used()));
        }
        if self.fold_ui.take_changed() {
            if let Some(fragment) = &mut self.fragment {
                fragment.refined = None;
            }
            self.viewport.fold_changed(self.document.document());
            self.cached_body = None;
            self.body_dirty = true;
            self.refiner = None;
            self.reading.view_changed();
        }
        if let Some(summary) = self.fold_ui.take_summary() {
            if let Some(fragment) = &mut self.fragment {
                fragment.refined = None;
            }
            self.viewport
                .set_fold_summary(self.document.document(), summary)?;
            self.cached_body = None;
            self.body_dirty = true;
            self.refiner = None;
            self.reading.view_changed();
        }
        // An old acknowledged frame may remain on screen while a current
        // source/width prefix is checked. Do not publish an intermediate cut
        // through a newly extended EGC or a prefix exceeding the new row cap.
        if self.fold_ui.prefix_validation_pending() {
            return Ok(FrameCommit {
                status: Some(ViewportFrameStatus::BudgetExhausted),
                metrics: FrameMetrics {
                    layout_budget: budget.used(),
                    frame_elapsed: started.elapsed(),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        if self.pending_sidebar_activate {
            self.activate_sidebar_selection(budget)?;
            if self.pending_sidebar_activate {
                return Ok(FrameCommit {
                    status: Some(ViewportFrameStatus::BudgetExhausted),
                    metrics: FrameMetrics {
                        layout_budget: budget.used(),
                        frame_elapsed: started.elapsed(),
                        ..FrameMetrics::default()
                    },
                    ..FrameCommit::default()
                });
            }
        }
        let folding_status = self.fold_ui.active() || self.viewport.folds().preview().is_some();
        let rows = if self.reading.status_visible() || folding_status || self.viewport.following() {
            self.geometry.rows.saturating_sub(1).max(1)
        } else {
            self.geometry.rows
        };
        if (self.reading.status_visible() || folding_status || self.fold_status_rows)
            && self.viewport.viewport_rows() != rows
        {
            self.viewport.set_viewport_rows(rows)?;
        }
        self.fold_status_rows = folding_status;
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
        let reading_first = self.reading_first || self.reading.copy_pending();
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
        if let Some(id) = self
            .fragment
            .as_ref()
            .filter(|p| !p.waiting || p.cancelled)
            .map(|p| p.id)
        {
            let result = if self.fragment.as_ref().is_some_and(|p| p.cancelled)
                || self.link_focus != Some(id)
            {
                let p = self.fragment.as_mut().unwrap();
                if p.activity
                    .as_mut()
                    .is_none_or(|a| a.retire_continuation(budget) == Completion::Covered)
                {
                    self.fragment = None;
                }
                None
            } else {
                Some(self.continue_fragment(id, budget)?)
            };
            if let Some(result) = result {
                self.set_notice(
                    result
                        .status
                        .shows_notice()
                        .then(|| result.status.message()),
                );
            }
        }
        if let Some(position) = self.pending_restore
            && self.restore_navigation_position(position, budget)?
                == HistoryRestoreStatus::BudgetExhausted
        {
            return Ok(FrameCommit {
                status: Some(ViewportFrameStatus::BudgetExhausted),
                metrics: FrameMetrics {
                    layout_budget: budget.used(),
                    frame_elapsed: started.elapsed(),
                    ..Default::default()
                },
                ..Default::default()
            });
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
        if self.reading.copy_pending() && self.presenter.wire.is_none() {
            return Ok(FrameCommit {
                status: Some(ViewportFrameStatus::BudgetExhausted),
                bytes: clipboard_bytes,
                metrics: FrameMetrics {
                    layout_budget: budget.used(),
                    output_bytes: clipboard_bytes,
                    clipboard_bytes,
                    frame_elapsed: started.elapsed(),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        let mut commit = self.commit_frame_started(writer, &frame, started, budget)?;
        commit.metrics.layout_budget = budget.used();
        commit.metrics.clipboard_bytes = clipboard_bytes;
        commit.metrics.output_bytes += clipboard_bytes;
        commit.bytes += clipboard_bytes;
        Ok(commit)
    }

    pub fn wire_pending(&self) -> bool {
        self.presenter.wire_pending()
    }
    pub fn abort_output<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        budget: &mut Budget,
    ) -> Result<bool, FrameError> {
        let Some(tx) = self.presenter.wire.as_mut() else {
            return Ok(self.viewport.shared_links().retire_initialization(budget));
        };
        tx.cancel();
        let done = tx.advance(writer, budget)? == Progress::Complete;
        if done {
            self.presenter.wire = None;
            self.presenter.sent = None;
            self.presenter.full_repaint = true;
        }
        Ok(done && self.viewport.shared_links().retire_initialization(budget))
    }
    pub fn commit_frame_budgeted<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        frame: &ViewportFrame,
        budget: &mut Budget,
    ) -> Result<FrameCommit, FrameError> {
        self.commit_frame_started(writer, frame, Instant::now(), budget)
    }
    pub fn commit_empty_frame_budgeted<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        budget: &mut Budget,
    ) -> Result<FrameCommit, FrameError> {
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
        self.commit_frame_started(writer, &frame, Instant::now(), budget)
    }

    pub fn commit_frame<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        frame: &ViewportFrame,
    ) -> Result<FrameCommit, FrameError> {
        self.commit_frame_started(
            writer,
            frame,
            Instant::now(),
            &mut Budget::new(WorkLimit::unlimited()),
        )
    }

    fn commit_frame_started<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        frame: &ViewportFrame,
        started: Instant,
        budget: &mut Budget,
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
        let fold_status = self
            .fold_ui
            .status(self.geometry.content_width)
            .or_else(|| {
                self.viewport
                    .folds()
                    .preview()
                    .and_then(|id| {
                        self.viewport
                            .folds()
                            .fold_candidate(self.document.document(), id)
                            .ok()
                            .flatten()
                    })
                    .map(|_| "preview | Esc: restore".to_owned())
            });
        // Publication is distinct from wire acknowledgement. Reuse the finite
        // logical back frame while a micro-budget writer drains its descriptor.
        self.cached_body = Some(frame.clone());
        self.body_dirty = false;
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
                notice: fold_status
                    .as_deref()
                    .or(reading_status.as_deref())
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
                    .flatten()
                    .or(match self.object_focus {
                        Some(VisibleObject::Fold(_, source)) => Some(source),
                        _ => None,
                    }),
                search: self.reading.current_match().map(|hit| hit.source),
            },
            started,
            budget,
        )?;
        if commit.published {
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
        self.commit_frame_started(
            writer,
            &frame,
            started,
            &mut Budget::new(WorkLimit::unlimited()),
        )
    }

    fn prepare_sidebar(&mut self, budget: &mut Budget) -> Result<(), FrameError> {
        self.ensure_sidebar_model()?;
        if self.sidebar_visible {
            let doc = self.document.document();
            let source = self.viewport.top_source(doc).unwrap_or(0);
            if let Some(sidebar) = self.sidebar.as_mut() {
                sidebar.advance(
                    doc,
                    source,
                    usize::from(self.geometry.rows),
                    usize::from(self.sidebar_width.saturating_sub(1)),
                    budget,
                )?;
            }
        }
        Ok(())
    }

    fn ensure_sidebar_model(&mut self) -> Result<(), FrameError> {
        if self.sidebar_visible && self.sidebar.is_none() {
            self.sidebar = Some(Directory::new(
                self.document.document(),
                self.viewport.shared_links(),
            ));
        }
        Ok(())
    }

    fn apply_sidebar_update(&mut self, update: &Update) -> Result<(), FrameError> {
        if let Some(sidebar) = self.sidebar.as_mut() {
            sidebar.apply_update(self.document.document(), update);
        }
        Ok(())
    }

    pub fn ensure_sidebar_selection(&mut self) -> Option<HeadingId> {
        self.sidebar_selection()
    }

    pub fn cancel_sidebar_activation(&mut self) {
        self.pending_sidebar_activate = false;
    }
    pub fn sidebar_event(&mut self, event: DirectoryEvent) -> bool {
        let changed = self.sidebar_visible && self.sidebar.as_mut().is_some_and(|s| s.event(event));
        if changed {
            self.cancel_sidebar_activation();
        }
        changed
    }
    pub fn sidebar_move_selection(&mut self, delta: isize) -> bool {
        self.sidebar_event(DirectoryEvent::Move(delta))
    }
    pub fn sidebar_selection_home(&mut self) -> bool {
        self.sidebar_event(DirectoryEvent::Home)
    }
    pub fn sidebar_selection_end(&mut self) -> bool {
        self.sidebar_event(DirectoryEvent::End)
    }

    pub fn activate_sidebar_selection(&mut self, budget: &mut Budget) -> Result<bool, FrameError> {
        self.prepare_sidebar(budget)?;
        if self
            .sidebar
            .as_ref()
            .is_some_and(|s| s.status == crate::sidebar::directory::NameStatus::Stale)
        {
            self.pending_sidebar_activate = false;
            return Ok(false);
        }
        if self.sidebar_selection().is_some()
            || self
                .sidebar
                .as_ref()
                .is_some_and(Directory::selection_pending)
        {
            self.reading.pause_search(&self.viewport.shared_links());
            if self.pending_search_target.take().is_some() {
                self.pending_jump = None;
                self.refiner = None;
            }
        }
        if self
            .sidebar
            .as_ref()
            .is_some_and(Directory::selection_pending)
        {
            self.pending_sidebar_activate = true;
            return Ok(false);
        }
        if !budget.charge(mdtui3::WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            self.pending_sidebar_activate = self.sidebar_selection().is_some();
            return Ok(false);
        }
        self.pending_sidebar_activate = false;
        let Some(selection) = self.ensure_sidebar_selection() else {
            return Ok(false);
        };
        let document = self.document.document();
        let Some(heading) = document.headings().get(selection).copied() else {
            return Ok(false);
        };
        // Source refinement performs paid Reveal first. Directory collapse
        // wishes remain independent of body fold choices.
        Ok(matches!(
            self.refine_source_to_top(heading.source.start, budget)?,
            RefineJump::Ready(true)
        ))
    }

    pub fn fold_hits(&self) -> &[FoldHit] {
        &self.fold_hits
    }

    pub fn has_visible_objects(&self) -> bool {
        !self.objects.is_empty()
    }
    pub fn enter_object_focus(&mut self, forward: bool) -> bool {
        let object = if forward {
            self.objects.first()
        } else {
            self.objects.last()
        }
        .copied();
        if let Some(object) = object {
            self.object_focus = Some(object);
            self.link_focus = match object {
                VisibleObject::Link(id) => Some(id),
                _ => None,
            };
            return true;
        }
        false
    }
    pub fn cycle_object_focus(&mut self, forward: bool) -> bool {
        if self.objects.is_empty() {
            return false;
        }
        let Some(index) = self
            .objects
            .iter()
            .position(|o| Some(*o) == self.object_focus)
        else {
            return self.enter_object_focus(forward);
        };
        let next = if forward {
            (index + 1) % self.objects.len()
        } else {
            (index + self.objects.len() - 1) % self.objects.len()
        };
        let object = self.objects[next];
        if self.object_focus == Some(object) {
            return false;
        }
        self.object_focus = Some(object);
        self.link_focus = match object {
            VisibleObject::Link(id) => Some(id),
            _ => None,
        };
        true
    }
    pub fn activate_focused_fold(&mut self) -> bool {
        if self.viewport.checkpoints().maintenance_in_progress() {
            return false;
        }
        let Some(VisibleObject::Fold(id, _)) = self.object_focus else {
            return false;
        };
        if self.object_version != self.viewport.folds().version() {
            return false;
        }
        self.fold_ui.activate(id);
        true
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

    pub fn focus_link_at(&mut self, row: u16, col: u16) -> bool {
        let Some(id) = self
            .presenter
            .screen()
            .front_cell(row, col)
            .and_then(|c| c.link_id())
        else {
            return false;
        };
        if !self.links.as_ref().is_some_and(|t| t.contains(id)) {
            return false;
        }
        self.link_focus = Some(id);
        self.object_focus = Some(VisibleObject::Link(id));
        true
    }
    pub fn focus_fold_at(&mut self, row: u16, col: u16) -> bool {
        if self.viewport.checkpoints().maintenance_in_progress() {
            return false;
        }
        let Some(hit) = self
            .fold_hits
            .iter()
            .find(|hit| {
                hit.row == row
                    && hit.x0 <= col
                    && col < hit.x1
                    && hit.epoch == self.viewport.folds().version()
            })
            .copied()
        else {
            return false;
        };
        self.object_focus = Some(VisibleObject::Fold(hit.id, hit.hidden_envelope));
        self.link_focus = None;
        self.cancel_fragment();
        true
    }
    pub fn clear_link_focus(&mut self) -> bool {
        let object = self.object_focus.take().is_some();
        self.link_focus.take().is_some() || object
    }

    pub fn advance_anchor_index(
        &mut self,
        budget: &mut Budget,
    ) -> Result<AnchorBuildStatus, FrameError> {
        self.anchors_started = true;
        let document = self.document.document();
        let links = self.viewport.shared_links();
        if !links.prepare(document, budget)? {
            return Ok(AnchorBuildStatus::BudgetExhausted);
        }
        if self.anchor_lease.is_none() {
            self.anchor_lease = Some(links.hold());
        }
        let result =
            links.with_resolver(|r| self.slugs.advance_with_resolver(document, r, budget))?;
        if result == Completion::Covered {
            if links.with_resolver(|r| r.style_memo_entries() == 0) {
                self.anchor_lease = None;
            }
            Ok(AnchorBuildStatus::Complete)
        } else {
            Ok(AnchorBuildStatus::BudgetExhausted)
        }
    }

    pub fn slug_session_stats(&self) -> mdtui3::SlugSessionStats {
        let mut stats = self.slugs.stats();
        self.viewport.shared_links().with_resolver(|r| {
            stats.canonical_style_work = r.style_work();
            stats.resolver_entries += r.memo_entries();
            stats.owned_capacity_bytes += r.capacity_bytes();
        });
        stats
    }
    pub fn fragment_refinement_pending(&self) -> bool {
        self.fragment.as_ref().is_some_and(|p| p.target.is_some()) && self.refiner.is_some()
    }
    /// Request-owned storage only. The shared renderer/resolver and source
    /// refiner retain their existing window stores, separately owned by runtime.
    pub fn fragment_capacity_bytes(&self) -> usize {
        self.fragment.as_ref().map_or(0, |p| {
            std::mem::size_of::<PendingFragment>()
                + p.decoded.capacity()
                + p.request
                    .as_ref()
                    .map_or(0, CurrentFragment::capacity_bytes)
                + p.activity
                    .as_ref()
                    .map_or(0, mdtui3::LinkActivity::continuation_capacity)
        })
    }
    pub fn retire_slug_session(&mut self, b: &mut Budget) -> Result<Completion, FrameError> {
        if let Some(p) = &mut self.fragment
            && let Some(a) = &mut p.activity
            && a.retire_continuation(b) != Completion::Covered
        {
            return Ok(Completion::BudgetExhausted);
        }
        self.fragment = None;
        self.anchor_lease = None;
        let result = self.slugs.retire(self.document.document(), b)?;
        if result != Completion::Covered {
            return Ok(result);
        }
        Ok(self.viewport.shared_links().retire_current(b))
    }
    pub fn fragment_completion(&self) -> (u64, Option<FragmentActivation>) {
        (self.fragment_serial, self.fragment_success)
    }
    pub fn cancel_fragment(&mut self) {
        if let Some(p) = &mut self.fragment {
            p.cancelled = true;
        }
    }
    pub fn activate_focused_link(
        &mut self,
        budget: &mut Budget,
    ) -> Result<FragmentActivation, FrameError> {
        let Some(id) = self.link_focus else {
            return Ok(FragmentActivation::status(FragmentActivationStatus::NoLink));
        };
        self.continue_fragment(id, budget)
    }
    fn continue_fragment(
        &mut self,
        id: LinkId,
        budget: &mut Budget,
    ) -> Result<FragmentActivation, FrameError> {
        use FragmentActivationStatus as S;
        let links = self.viewport.shared_links();
        let document = self.document.document();
        if !links.prepare(document, budget)? {
            return Ok(FragmentActivation::status(S::BudgetExhausted));
        }
        if self
            .fragment
            .as_ref()
            .is_some_and(|p| p.id != id || p.cancelled)
        {
            let p = self.fragment.as_mut().unwrap();
            if let Some(activity) = &mut p.activity
                && activity.retire_continuation(budget) != Completion::Covered
            {
                return Ok(FragmentActivation::status(S::BudgetExhausted));
            }
            self.fragment = None;
        }
        if self.fragment.is_none() {
            if !budget.charge(WorkLimit {
                objects: 1,
                queries: 1,
                ..Default::default()
            }) {
                return Ok(FragmentActivation::status(S::BudgetExhausted));
            }
            let Some(activity) = document.link_activity(id) else {
                return Ok(FragmentActivation::status(S::Invalid));
            };
            self.fragment = Some(PendingFragment {
                id,
                revision: document.revision(),
                activity: Some(activity),
                active: false,
                destination: None,
                decoded: String::with_capacity(mdtui3::MAX_DESTINATION_BYTES),
                request: None,
                target: None,
                refined: None,
                cancelled: false,
                waiting: false,
                _lease: links.hold(),
            });
        }
        let p = self.fragment.as_mut().unwrap();
        if p.revision != document.revision() {
            let retained = links.with_resolver(|r| r.can_rebind(document, p.revision));
            if let Some(activity) = &mut p.activity {
                if retained {
                    links.with_resolver(|r| activity.rebind_with(document, r))?;
                } else {
                    if activity.retire_continuation(budget) != Completion::Covered {
                        return Ok(FragmentActivation::status(S::BudgetExhausted));
                    }
                    p.activity = None;
                }
            }
            if !retained {
                p.active = false;
            }
            p.refined = None;
            p.target = None;
            p.revision = document.revision();
            if p.destination
                .as_ref()
                .is_some_and(|d| !d.is_current(document, id))
            {
                p.destination = None;
                p.decoded = String::with_capacity(mdtui3::MAX_DESTINATION_BYTES);
                p.request = None;
                p.target = None;
            }
        }
        if !p.active {
            if p.activity.is_none() {
                p.activity = document.link_activity(id);
            }
            let Some(activity) = &mut p.activity else {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::Invalid));
            };
            let result = links.with_resolver(|r| activity.advance_with(document, r, budget))?;
            let Some(result) = result else {
                return Ok(FragmentActivation::status(S::BudgetExhausted));
            };
            if activity.retire_continuation(budget) != Completion::Covered {
                return Ok(FragmentActivation::status(S::BudgetExhausted));
            }
            p.activity = None;
            if result != mdtui3::LinkDisposition::Active {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::Invalid));
            }
            p.active = true;
        }
        if p.destination.is_none() {
            let Some(raw) = document.link_destination(id) else {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::Invalid));
            };
            if raw.is_empty() {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::Invalid));
            }
            if !raw.starts_with('#') {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::External));
            }
            if raw.len() > mdtui3::MAX_DESTINATION_BYTES {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::Invalid));
            }
            p.destination = document.link_destination_task(id);
        }
        if p.request.is_none() {
            loop {
                match p.destination.as_mut().unwrap().advance(document, budget)? {
                    Err(_) => return Ok(FragmentActivation::status(S::BudgetExhausted)),
                    Ok(Some(chunk)) => p.decoded.push_str(chunk.text),
                    Ok(None) => break,
                }
            }
            let decoded = Arc::new(std::mem::take(&mut p.decoded));
            p.request = Some(self.slugs.fragment_owned(decoded)?);
        }
        self.anchors_started = true;
        // Resolve once per revision, then spend later micro-frames on Reveal
        // and local refinement. The current owner is still revalidated before
        // committing the prepared answer below.
        let target = if let Some(target) = p.target {
            target
        } else {
            let result = links.with_resolver(|r| {
                self.slugs
                    .fragment_with_resolver(document, r, p.request.as_mut().unwrap(), budget)
            });
            let result = match result {
                Ok(v) => v,
                Err(LayoutError::InvalidRange) => {
                    self.fragment = None;
                    return Ok(FragmentActivation::status(S::Invalid));
                }
                Err(e) => return Err(e.into()),
            };
            let target = match result {
                FragmentStep::Resolved(t) => t,
                FragmentStep::BudgetExhausted | FragmentStep::IndexIncomplete => {
                    return Ok(FragmentActivation::status(
                        if self.slugs.is_current(document) {
                            S::BudgetExhausted
                        } else {
                            S::PendingIndex
                        },
                    ));
                }
                FragmentStep::Unresolved => {
                    p.waiting = true;
                    return Ok(FragmentActivation::status(S::PendingDocument));
                }
                FragmentStep::NotFound => {
                    self.fragment = None;
                    return Ok(FragmentActivation::status(S::NotFound));
                }
            };
            p.target = Some(target);
            target
        };
        let refined = p.refined.take();
        let refined = if let Some(target) = refined {
            RefineTarget::Ready(target)
        } else {
            self.refine_target(target.source, budget)?
        };
        let moved = match refined {
            RefineTarget::Pending => return Ok(FragmentActivation::status(S::BudgetExhausted)),
            RefineTarget::NotFound => {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::NotFound));
            }
            RefineTarget::Failed => {
                self.fragment = None;
                return Ok(FragmentActivation::status(S::Failed));
            }
            RefineTarget::Empty => false,
            RefineTarget::Ready(refined) => {
                let p = self.fragment.as_mut().unwrap();
                // The successful refiner may have consumed this frame's last
                // unit. Keep its answer, then revalidate the current owner on
                // the next frame before committing any navigation/history.
                let current = links.with_resolver(|r| {
                    self.slugs.fragment_with_resolver(
                        self.document.document(),
                        r,
                        p.request.as_mut().unwrap(),
                        budget,
                    )
                })?;
                if current != FragmentStep::Resolved(target) {
                    p.refined = Some(refined);
                    return Ok(FragmentActivation::status(S::BudgetExhausted));
                }
                self.commit_refined_source(refined)?
            }
        };
        self.fragment = None;
        if links.enable_fragment_titles() {
            self.viewport.refresh_fragment_titles();
            self.cached_body = None;
        }
        if self.anchor_lease.is_none() && links.with_resolver(|r| r.style_memo_entries() != 0) {
            self.anchor_lease = Some(links.hold());
        }
        let result = FragmentActivation::moved(target, moved);
        self.fragment_serial += 1;
        self.fragment_success = Some(result);
        self.body_dirty = true;
        Ok(result)
    }
    fn commit_refined_source(&mut self, target: RefinedTarget) -> Result<bool, FrameError> {
        self.pending_jump = None;
        self.pending_sidebar_activate = false;
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
        Ok(moved)
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
                Ok(RefineJump::Ready(self.commit_refined_source(target)?))
            }
            RefineTarget::Empty => Ok(RefineJump::Ready(false)),
            RefineTarget::Pending => {
                self.pending_jump = Some(source);
                Ok(RefineJump::Pending)
            }
            RefineTarget::NotFound => {
                self.pending_jump = None;
                self.pending_sidebar_activate = false;
                Ok(RefineJump::NotFound)
            }
            RefineTarget::Failed => {
                self.pending_jump = None;
                self.pending_sidebar_activate = false;
                Ok(RefineJump::Failed)
            }
        }
    }

    fn refine_target(
        &mut self,
        source: usize,
        budget: &mut Budget,
    ) -> Result<RefineTarget, FrameError> {
        if self.viewport.checkpoints().maintenance_in_progress() {
            return Ok(RefineTarget::Pending);
        }
        let document = self.document.document();
        if document.tile_count() == 0 {
            return Ok(RefineTarget::Empty);
        }
        if self.revealed != Some((document.revision(), self.viewport.folds().version(), source)) {
            if self
                .fold_jump
                .as_ref()
                .is_none_or(|(old, _)| *old != source)
            {
                self.fold_jump = Some((
                    source,
                    mdtui3::FoldNavigation::new(
                        document,
                        self.viewport.folds(),
                        &self.navigator,
                        mdtui3::NavAction::Source(source),
                        mdtui3::FoldNavigationPolicy::Reveal,
                        u64::from(self.viewport.viewport_rows()),
                    )
                    .map_err(ViewportError::Layout)?,
                ));
            }
            let progress = self
                .fold_jump
                .as_mut()
                .unwrap()
                .1
                .advance(document, self.viewport.folds_mut(), budget)
                .map_err(ViewportError::Layout)?;
            if progress.change.is_some() {
                self.viewport.fold_changed(document);
                self.cached_body = None;
                self.body_dirty = true;
                self.reading.view_changed();
            }
            if progress.completion == mdtui3::Completion::BudgetExhausted {
                return Ok(RefineTarget::Pending);
            }
            self.fold_jump = None;
            self.revealed = Some((document.revision(), self.viewport.folds().version(), source));
        }
        let lookup_source = source;
        let viewport_config = self.viewport.config();
        let config = RefineConfig {
            width: self.geometry.content_width,
            max_tiles: viewport_config.max_tiles,
            max_replay_bytes: viewport_config.max_replay_bytes,
            row_slack: viewport_config.row_slack,
        };
        let focus = self
            .pending_search_target
            .and_then(|_| self.reading.current_math_focus());
        if self.refiner.as_ref().is_some_and(|refiner| {
            !refiner.matches(document)
                || refiner.source() != lookup_source
                || refiner.config() != config
                || self.refiner_focus != focus
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
            let checkpoints = self.viewport.layout_owner_mut();
            if self.refiner.is_none() {
                let Ok(mut refiner) = SourceRefiner::new(
                    document,
                    checkpoints.heights(),
                    checkpoints,
                    lookup_source,
                    config,
                ) else {
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
                self.refiner_focus = focus;
            }
            let Ok(progress) = self
                .refiner
                .as_mut()
                .expect("created refiner")
                .advance_owned(document, checkpoints, budget)
            else {
                self.refiner = None;
                return Ok(RefineTarget::Failed);
            };
            progress
        };
        match progress {
            RefineProgress::Ready(target) => {
                self.refiner = None;
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
        let mut ordered = Vec::new();
        self.fold_hits.clear();
        let screen = self.presenter.screen();
        for row in 0..screen.rows() {
            for col in self.content_x..self.content_x + self.geometry.content_width {
                if let Some(cell) = screen.reading_cell(row, col) {
                    let object = if let Some(id) = cell.link_id() {
                        Some(VisibleObject::Link(id))
                    } else {
                        cell.source().and_then(|source| {
                            self.viewport
                                .shown_folds
                                .iter()
                                .find(|p| p.candidate.hidden == source)
                                .map(|p| {
                                    let epoch = self.viewport.folds().version();
                                    if let Some(last) = self.fold_hits.last_mut().filter(|hit| {
                                        hit.row == row
                                            && hit.x1 == col
                                            && hit.id == p.candidate.id
                                            && hit.hidden_envelope == source
                                    }) {
                                        last.x1 = col + u16::from(cell.width().max(1));
                                    } else {
                                        self.fold_hits.push(FoldHit {
                                            id: p.candidate.id,
                                            owner_tile: p.owner,
                                            component_start: p.candidate.hidden.start,
                                            registered: p.candidate.source,
                                            hidden_envelope: source,
                                            epoch,
                                            row,
                                            x0: col,
                                            x1: col + u16::from(cell.width().max(1)),
                                        });
                                    }
                                    VisibleObject::Fold(p.candidate.id, source)
                                })
                        })
                    };
                    if let Some(object) = object
                        && !ordered.contains(&object)
                    {
                        ordered.push(object);
                    }
                }
            }
        }
        self.objects = ordered;
        if self
            .object_focus
            .is_some_and(|o| !self.objects.contains(&o))
        {
            self.object_focus = None;
        }
        self.object_version = self.viewport.folds().version();
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
    wire: Option<Transaction>,
    sent: Option<crate::screen::ScreenSnapshot>,
    sent_revision: u64,
    reset_pending: bool,
}

impl FramePresenter {
    fn new(rows: u16, cols: u16, options: OutputOptions) -> Result<Self, FrameError> {
        let screen = Screen::new(rows, cols, ContentArea::new(0, cols))?;
        Ok(Self {
            screen,
            options,
            first_clear_sent: false,
            full_repaint: false,
            wire: None,
            sent: None,
            sent_revision: 0,
            reset_pending: false,
        })
    }

    fn screen(&self) -> &Screen {
        &self.screen
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), FrameError> {
        if let Some(tx) = self.wire.as_mut() {
            tx.cancel();
            self.reset_pending = true;
        }
        self.screen.resize(rows, cols, ContentArea::new(0, cols))?;
        self.full_repaint = true;
        Ok(())
    }

    fn reset_baseline(&mut self) -> Result<(), FrameError> {
        if let Some(tx) = self.wire.as_mut() {
            tx.cancel();
            self.reset_pending = true;
        }
        self.screen.resize(
            self.screen.rows(),
            self.screen.cols(),
            self.screen.content_area(),
        )?;
        self.full_repaint = true;
        Ok(())
    }

    fn wire_pending(&self) -> bool {
        self.wire.as_ref().is_some_and(Transaction::pending)
    }
    fn open_cell(&self, document: &Document) -> Option<OpenCell> {
        if document.is_finished() {
            return None;
        }
        for row in 0..self.screen.rows() {
            for column in 0..self.screen.cols() {
                let cell = self
                    .screen
                    .zero_cell(row, column)
                    .filter(|c| c.text().ends_at_source(document.source().len()))
                    .or_else(|| self.screen.cell(row, column))?;
                if !cell.is_continuation()
                    && cell.text().is_indexed()
                    && cell.text().ends_at_source(document.source().len())
                {
                    return Some(OpenCell {
                        row,
                        column,
                        cell: cell.clone(),
                    });
                }
            }
        }
        None
    }
    fn commit<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        document: &Document,
        frame: &ViewportFrame,
        layout: FramePaintLayout<'_>,
        started: Instant,
        budget: &mut Budget,
    ) -> Result<FrameCommit, FrameError> {
        if self.wire.is_none()
            && !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            })
        {
            return Ok(FrameCommit::default());
        }
        self.paint(frame, layout)?;
        self.paint_link_focus(layout)?;
        self.paint_reading(layout)?;
        self.paint_notice(layout)?;
        self.screen.publish_back();
        let candidate = self.open_cell(document);
        let cleared = !self.first_clear_sent;
        let full_repaint = self.full_repaint;
        let mut bytes = 0;
        let mut output = OutputStats::default();
        let mut rows_changed = 0;
        for _ in 0..2 {
            if let Some(tx) = self.wire.as_mut() {
                if tx.closed.as_ref().is_some_and(|closed| {
                    let current = if closed.cell.width() == 0 {
                        self.screen.zero_cell(closed.row, closed.column)
                    } else {
                        self.screen.cell(closed.row, closed.column)
                    };
                    current.is_none_or(|cell| !cell.same_visual(&closed.cell))
                }) {
                    tx.cancel();
                    self.reset_pending = true;
                    self.full_repaint = true;
                }
                if tx.open.is_none()
                    && self
                        .sent
                        .as_ref()
                        .is_some_and(|sent| self.screen.indexed_snapshot_changed(sent))
                {
                    tx.cancel();
                    self.reset_pending = true;
                    self.full_repaint = true;
                }
                if tx.open.is_some() {
                    if budget.remaining().queries == 0 || budget.remaining().objects == 0 {
                        return Ok(FrameCommit {
                            published: true,
                            bytes,
                            metrics: FrameMetrics {
                                output_bytes: bytes,
                                ..Default::default()
                            },
                            ..Default::default()
                        });
                    }
                    let old = tx.open.as_ref().unwrap();
                    let same_position = candidate
                        .as_ref()
                        .is_some_and(|next| next.row == old.row && next.column == old.column);
                    if same_position {
                        if tx.extend(candidate.clone().unwrap(), budget) {
                            self.sent_revision = frame.revision;
                            tx.report_open_cut();
                        } else {
                            tx.cancel();
                            self.reset_pending = true;
                            self.full_repaint = true;
                        }
                    } else {
                        let next = if old.cell.width() == 0 {
                            self.screen.zero_cell(old.row, old.column)
                        } else {
                            self.screen.cell(old.row, old.column)
                        }
                        .map(|cell| OpenCell {
                            row: old.row,
                            column: old.column,
                            cell: cell.clone(),
                        });
                        // The final Extend and EOF/newline can arrive before
                        // an old cut is ACKed. Extend that same proven prefix
                        // before closing, without repainting its old bytes.
                        if let Some(next) = next
                            && tx.extend(next.clone(), budget)
                        {
                            tx.close();
                            self.sent_revision = frame.revision;
                            if let Some(sent) = self.sent.as_mut() {
                                sent.acknowledge_cell(next.row, next.column, next.cell);
                            }
                        } else {
                            tx.cancel();
                            self.reset_pending = true;
                            self.full_repaint = true;
                        }
                    }
                }
            } else {
                let diff = if self.full_repaint {
                    full_screen_diff(&self.screen)
                } else {
                    diff_screen(&self.screen)
                };
                rows_changed = diff.rows_changed;
                // Hold only an EGC actually present in this diff. An unchanged
                // open cut is retained by its existing transaction.
                let open = candidate.clone().filter(|open| {
                    diff.runs.iter().any(|run| {
                        run.row == open.row
                            && run.start_col <= open.column
                            && open.column < run.end_col()
                    })
                });
                self.wire = Some(Transaction::prepare(
                    document,
                    &self.screen,
                    &diff,
                    self.options,
                    !self.first_clear_sent,
                    open,
                )?);
                self.sent = Some(self.screen.snapshot_back());
                self.sent_revision = frame.revision;
            }
            let tx = self.wire.as_mut().unwrap();
            let before = tx.bytes;
            let progress = match tx.advance(writer, budget) {
                Ok(progress) => progress,
                Err(error) => {
                    let _ = writer.write_all(crate::render::wire::cleanup(self.options));
                    return Err(error.into());
                }
            };
            let written = tx.bytes - before;
            bytes += written;
            output = tx.stats;
            if tx.cancelled {
                self.reset_pending = true;
                self.full_repaint = true;
            }
            match progress {
                Progress::Pending => {
                    return Ok(FrameCommit {
                        published: true,
                        bytes,
                        output,
                        metrics: FrameMetrics {
                            output_bytes: bytes,
                            body_bytes: bytes,
                            frame_elapsed: started.elapsed(),
                            ..Default::default()
                        },
                        ..Default::default()
                    });
                }
                Progress::OpenCut => {
                    if let Some(sent) = self.sent.take() {
                        self.screen.acknowledge(sent);
                    }
                    if let Some(open) = &tx.open {
                        self.screen
                            .acknowledge_cell(open.row, open.column, open.cell.clone())?;
                    }
                    self.first_clear_sent = true;
                    self.full_repaint = false;
                    return Ok(FrameCommit {
                        published: true,
                        status: Some(frame.status),
                        committed: true,
                        rows: frame.rows,
                        bytes,
                        output,
                        cleared,
                        first_frame: cleared,
                        full_repaint,
                        metrics: FrameMetrics {
                            layout_budget: frame.stats.layout_budget,
                            planned_tiles: frame.stats.planned_tiles,
                            visible_tiles: frame.stats.visible_tiles,
                            recovery_tiles: frame.stats.replay_tiles,
                            diff_rows: rows_changed,
                            output_bytes: bytes,
                            body_bytes: bytes,
                            layout: frame.stats.layout,
                            frame_elapsed: started.elapsed(),
                            ..Default::default()
                        },
                    });
                }
                Progress::Complete => {
                    let closed = tx.closed.clone();
                    let mut latest = self.screen.back_matches_front();
                    if let Some(sent) = self.sent.take() {
                        latest = self.screen.snapshot_matches_back(&sent);
                        if !self.reset_pending {
                            self.screen.acknowledge(sent);
                        }
                    }
                    if let Some(closed) = closed
                        && !self.reset_pending
                    {
                        self.screen
                            .acknowledge_cell(closed.row, closed.column, closed.cell)?;
                        latest = self.screen.back_matches_front();
                    }
                    self.wire = None;
                    self.first_clear_sent = true;
                    if self.reset_pending || self.sent_revision != frame.revision || !latest {
                        self.reset_pending = false;
                        continue;
                    }
                    self.full_repaint = false;
                    return Ok(FrameCommit {
                        published: true,
                        status: Some(frame.status),
                        committed: true,
                        rows: frame.rows,
                        bytes,
                        output,
                        cleared,
                        first_frame: cleared,
                        full_repaint,
                        metrics: FrameMetrics {
                            layout_budget: frame.stats.layout_budget,
                            planned_tiles: frame.stats.planned_tiles,
                            visible_tiles: frame.stats.visible_tiles,
                            recovery_tiles: frame.stats.replay_tiles,
                            diff_rows: rows_changed,
                            output_bytes: bytes,
                            body_bytes: bytes,
                            layout: frame.stats.layout,
                            frame_elapsed: started.elapsed(),
                            ..Default::default()
                        },
                    });
                }
            }
        }
        Ok(FrameCommit {
            published: true,
            bytes,
            output,
            metrics: FrameMetrics {
                output_bytes: bytes,
                body_bytes: bytes,
                frame_elapsed: started.elapsed(),
                ..Default::default()
            },
            ..Default::default()
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
        sidebar: Option<&Directory>,
    ) -> Result<(), FrameError> {
        let border_column = sidebar_width - 1;
        let text_width = usize::from(border_column);
        for row in 0..self.screen.rows() {
            self.screen.write_cell(
                row,
                border_column,
                Cell::new("│", StyleRole::TABLE_BORDER, 1, None, None)?,
            )?;
            if usize::from(row) < HEADER_ROWS {
                if let Some(sidebar) = sidebar {
                    let message = if row == 0 {
                        sidebar.breadcrumb_text()
                    } else {
                        sidebar.notice()
                    };
                    let text = clip_sidebar_text(&message, text_width);
                    let mut cursor = TextCursor::new(0);
                    for grapheme in graphemes(&text) {
                        cursor.place(
                            &mut self.screen,
                            row,
                            grapheme,
                            StyleRole::META,
                            None,
                            None,
                        )?;
                    }
                }
                continue;
            }
            let index = usize::from(row) - HEADER_ROWS;
            let Some(entry) = sidebar.and_then(|sidebar| sidebar.entry(index)) else {
                continue;
            };
            let indent = sidebar
                .map_or(0, |s| s.depth(index))
                .saturating_mul(2)
                .min(text_width.saturating_sub(4));
            if indent >= text_width {
                continue;
            }
            let available = text_width.saturating_sub(indent + 2);
            let text = format!(
                "{} {}",
                sidebar.map_or(" ", |s| s.marker(index)),
                clip_sidebar_text(&entry.text, available)
            );
            let style = match sidebar {
                Some(sidebar)
                    if sidebar.current() == Some(entry.id)
                        && sidebar.selection() == Some(entry.id) =>
                {
                    StyleRole::SIDEBAR_CURRENT.with(StyleRole::SIDEBAR_SELECTION)
                }
                Some(sidebar) if sidebar.current() == Some(entry.id) => StyleRole::SIDEBAR_CURRENT,
                Some(sidebar) if sidebar.selection() == Some(entry.id) => {
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
                if let Some(zero) = self
                    .screen
                    .zero_cell(row, column)
                    .filter(|c| c.link_id() == Some(focused))
                {
                    self.screen.set_zero_style(
                        row,
                        column,
                        zero.style().with(StyleRole::LINK_FOCUS),
                    )?;
                }
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
                if let Some(zero) = self.screen.zero_cell(row, col)
                    && let Some(source) = zero.source()
                {
                    let mut style = zero.style();
                    for (range, role) in [
                        (layout.search, StyleRole::SEARCH),
                        (layout.selection, StyleRole::SELECTION),
                        (layout.cursor, StyleRole::CURSOR),
                    ] {
                        if range.is_some_and(|range| crate::reading::overlaps(source, range)) {
                            style = style.with(role);
                        }
                    }
                    self.screen.set_zero_style(row, col, style)?;
                }
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

fn sidebar_layout_for(cols: u16, visible: bool) -> (bool, u16, u16) {
    if !visible {
        return (false, 0, cols);
    }
    let Some(width) = crate::app::sidebar_width(cols) else {
        return (false, 0, cols);
    };
    (true, width, cols.saturating_sub(width))
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
        let cells: Vec<_> = (usize::from(content.x)..usize::from(content.right()))
            .flat_map(|at| {
                screen
                    .zero_cell(row, at as u16)
                    .cloned()
                    .into_iter()
                    .chain(std::iter::once(back[at].clone()))
            })
            .collect();
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
