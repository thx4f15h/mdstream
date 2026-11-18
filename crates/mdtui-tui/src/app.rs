use crate::render::frame::HistoryRestoreStatus;
use crossterm::event::{KeyCode, KeyModifiers};
use mdtui3::LinkId;

use crate::external::ExternalOpenOutcome;
use crate::fragment::FragmentActivation;
use crate::history::{HistoryPosition, HistoryState};
use crate::stream::{StreamEngine, StreamError};

pub const MIN_SIDEBAR_WIDTH: u16 = 20;
pub const MAX_SIDEBAR_WIDTH: u16 = 40;
pub const MIN_SPLIT_TERMINAL_WIDTH: u16 = MIN_SIDEBAR_WIDTH * 2;

#[must_use]
pub fn sidebar_width(terminal_cols: u16) -> Option<u16> {
    if terminal_cols < MIN_SPLIT_TERMINAL_WIDTH {
        return None;
    }
    let preferred = u16::try_from(u32::from(terminal_cols) * 3 / 10).unwrap_or(u16::MAX);
    Some(
        preferred
            .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
            .min(terminal_cols - MIN_SIDEBAR_WIDTH),
    )
}

#[must_use]
pub fn content_width(terminal_cols: u16, sidebar_visible: bool) -> u16 {
    if !sidebar_visible {
        return terminal_cols;
    }
    terminal_cols.saturating_sub(sidebar_width(terminal_cols).unwrap_or(0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Quit,
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Home,
    End,
    JumpEnd,
    FocusNext,
    FocusPrevious,
    ToggleSidebar,
    Activate,
    HistoryBack,
    HistoryForward,
    PanTable(isize),
    None,
}

#[must_use]
pub fn action_for_key(code: KeyCode, modifiers: KeyModifiers) -> KeyAction {
    match (code, modifiers) {
        (KeyCode::Char('q'), _) => KeyAction::Quit,
        (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            KeyAction::Quit
        }
        (KeyCode::Left, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            KeyAction::HistoryBack
        }
        (KeyCode::Right, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            KeyAction::HistoryForward
        }
        (KeyCode::Left, modifiers) => {
            KeyAction::PanTable(if modifiers.contains(KeyModifiers::CONTROL) {
                -8
            } else {
                -1
            })
        }
        (KeyCode::Right, modifiers) => {
            KeyAction::PanTable(if modifiers.contains(KeyModifiers::CONTROL) {
                8
            } else {
                1
            })
        }
        (KeyCode::Up, _) => KeyAction::LineUp,
        (KeyCode::Down, _) => KeyAction::LineDown,
        (KeyCode::PageUp, _) => KeyAction::PageUp,
        (KeyCode::PageDown, _) => KeyAction::PageDown,
        (KeyCode::Home, _) => KeyAction::Home,
        (KeyCode::End, modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            KeyAction::JumpEnd
        }
        (KeyCode::End, _) => KeyAction::End,
        (KeyCode::Tab, modifiers) if modifiers.contains(KeyModifiers::SHIFT) => {
            KeyAction::FocusPrevious
        }
        (KeyCode::Tab, _) => KeyAction::FocusNext,
        (KeyCode::BackTab, _) => KeyAction::FocusPrevious,
        (KeyCode::Char('s'), modifiers)
            if !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
        {
            KeyAction::ToggleSidebar
        }
        (KeyCode::Enter, _) => KeyAction::Activate,
        _ => KeyAction::None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FocusTarget {
    #[default]
    Body,
    Sidebar,
    Link,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppOutcome {
    pub quit: bool,
    pub redraw: bool,
    pub activated: Option<FocusTarget>,
    pub activated_link: Option<LinkId>,
    pub fragment_activation: Option<FragmentActivation>,
    pub external_open: Option<ExternalOpenOutcome>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    sidebar_visible: bool,
    focus: FocusTarget,
    history: HistoryState,
    last_fragment_activation: Option<FragmentActivation>,
    last_external_open: Option<ExternalOpenOutcome>,
    pending_jump_from: Option<HistoryPosition>,
    pending_fragment_from: Option<HistoryPosition>,
    fragment_serial: u64,
    pending_history: Option<(bool, HistoryPosition)>,
}

impl AppState {
    #[must_use]
    pub const fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    #[must_use]
    pub const fn focus(&self) -> FocusTarget {
        self.focus
    }

    #[must_use]
    pub const fn history(&self) -> &HistoryState {
        &self.history
    }

    #[must_use]
    pub const fn last_fragment_activation(&self) -> Option<FragmentActivation> {
        self.last_fragment_activation
    }

    #[must_use]
    pub const fn last_external_open(&self) -> Option<ExternalOpenOutcome> {
        self.last_external_open
    }

    pub fn handle_key(
        &mut self,
        engine: &mut StreamEngine,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<AppOutcome, StreamError> {
        if code == KeyCode::Char('z') && !engine.first_frame_done() {
            return Ok(AppOutcome::default());
        }
        if engine.runtime_mut().fold_menu_key(code) {
            engine.request_render();
            return Ok(AppOutcome {
                redraw: true,
                ..AppOutcome::default()
            });
        }
        if self.focus == FocusTarget::Sidebar {
            let editing = engine
                .runtime()
                .directory()
                .is_some_and(|s| s.editing().is_some());
            let plain = !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            let changed = if let Some(sidebar) = engine.runtime_mut().directory_mut() {
                if editing {
                    match code {
                        KeyCode::Esc => sidebar.cancel_edit(),
                        KeyCode::Enter => sidebar.submit(),
                        KeyCode::Backspace | KeyCode::Delete => sidebar.delete(),
                        KeyCode::Char(ch) if plain => sidebar.insert(ch),
                        _ => false,
                    }
                } else {
                    match code {
                        KeyCode::Char('/') if plain => sidebar.begin_edit(),
                        KeyCode::Char('c') if plain => {
                            sidebar.event(crate::sidebar::directory::DirectoryEvent::Current)
                        }
                        KeyCode::Char(ch @ '0'..='6') if plain => {
                            sidebar.set_level(ch as u8 - b'0')
                        }
                        KeyCode::Esc => sidebar.clear_query(),
                        KeyCode::Left if plain => {
                            sidebar.event(crate::sidebar::directory::DirectoryEvent::Left)
                        }
                        KeyCode::Right if plain => {
                            sidebar.event(crate::sidebar::directory::DirectoryEvent::Right)
                        }
                        _ => false,
                    }
                }
            } else {
                false
            };
            if changed {
                engine.runtime_mut().cancel_sidebar_activation();
                engine.request_render();
            }
            let reserved = plain
                && matches!(
                    code,
                    KeyCode::Char('/' | 'c' | '0'..='6') | KeyCode::Left | KeyCode::Right
                )
                || code == KeyCode::Esc;
            let global = matches!(code, KeyCode::Tab | KeyCode::BackTab)
                || (code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
                || (matches!(code, KeyCode::Left | KeyCode::Right)
                    && modifiers.contains(KeyModifiers::ALT));
            if (editing && !global) || reserved {
                return Ok(AppOutcome {
                    redraw: changed,
                    ..AppOutcome::default()
                });
            }
        }
        if self.focus == FocusTarget::Sidebar && code == KeyCode::Char('z') {
            self.pending_jump_from = None;
            self.pending_fragment_from = None;
            engine.runtime_mut().open_fold_menu(true);
            engine.request_render();
            return Ok(AppOutcome {
                redraw: true,
                ..AppOutcome::default()
            });
        }
        if code == KeyCode::Esc
            && self.focus == FocusTarget::Link
            && engine.runtime().reading().input().is_none()
        {
            self.focus = FocusTarget::Body;
            engine.clear_link_focus();
        }
        if self.focus != FocusTarget::Sidebar
            && !(matches!(code, KeyCode::Left | KeyCode::Right)
                && modifiers.contains(KeyModifiers::ALT))
            && engine
                .runtime_mut()
                .reading_key(code, modifiers, self.focus == FocusTarget::Body)?
        {
            if matches!(code, KeyCode::Char('z') | KeyCode::Esc) {
                self.pending_jump_from = None;
                self.pending_fragment_from = None;
            }
            engine.runtime_mut().cancel_sidebar_activation();
            engine.request_render();
            return Ok(AppOutcome {
                redraw: true,
                ..AppOutcome::default()
            });
        }
        self.handle_action(engine, action_for_key(code, modifiers))
    }

    pub fn handle_action(
        &mut self,
        engine: &mut StreamEngine,
        action: KeyAction,
    ) -> Result<AppOutcome, StreamError> {
        let mut outcome = AppOutcome::default();
        let previous_focus = self.focus;
        let normalized_redraw = self.normalize_focus(engine);
        let cancel = matches!(
            action,
            KeyAction::LineUp
                | KeyAction::LineDown
                | KeyAction::PageUp
                | KeyAction::PageDown
                | KeyAction::Home
                | KeyAction::HistoryBack
                | KeyAction::HistoryForward
        ) || (matches!(action, KeyAction::End | KeyAction::JumpEnd)
            && self.focus == FocusTarget::Sidebar)
            || (action == KeyAction::Activate && self.focus != FocusTarget::Body);
        let end_cancelled = cancel && engine.pause_following();
        if cancel {
            self.pending_jump_from = None;
            if action != KeyAction::Activate {
                engine.runtime_mut().cancel_fragment();
                self.pending_fragment_from = None;
            }
            self.pending_history = None;
        }
        match action {
            KeyAction::Quit => outcome.quit = true,
            KeyAction::PanTable(delta) => {
                if self.focus != FocusTarget::Sidebar {
                    outcome.redraw = engine.pan_table(delta)?;
                }
            }
            KeyAction::LineUp => {
                outcome.redraw = if self.focus == FocusTarget::Sidebar {
                    self.move_sidebar_selection(engine, -1)?
                } else {
                    self.scroll_up(engine, 1)?
                };
            }
            KeyAction::LineDown => {
                outcome.redraw = if self.focus == FocusTarget::Sidebar {
                    self.move_sidebar_selection(engine, 1)?
                } else {
                    self.scroll_down(engine, 1)?
                };
            }
            KeyAction::PageUp => {
                let rows = engine.runtime().geometry().rows;
                let page = isize::try_from(rows).unwrap_or(isize::MAX);
                outcome.redraw = if self.focus == FocusTarget::Sidebar {
                    self.move_sidebar_selection(engine, -page)?
                } else {
                    self.scroll_up(engine, u64::from(rows))?
                };
            }
            KeyAction::PageDown => {
                let rows = engine.runtime().geometry().rows;
                let page = isize::try_from(rows).unwrap_or(isize::MAX);
                outcome.redraw = if self.focus == FocusTarget::Sidebar {
                    self.move_sidebar_selection(engine, page)?
                } else {
                    self.scroll_down(engine, u64::from(rows))?
                };
            }
            KeyAction::Home => {
                outcome.redraw = if self.focus == FocusTarget::Sidebar {
                    let changed = engine.sidebar_selection_home();
                    if changed {
                        engine.request_render();
                    }
                    changed
                } else {
                    self.scroll_up(engine, u64::MAX)?
                };
            }
            KeyAction::End | KeyAction::JumpEnd => {
                outcome.redraw = if self.focus == FocusTarget::Sidebar {
                    let changed = engine.sidebar_selection_end();
                    if changed {
                        engine.request_render();
                    }
                    changed
                } else {
                    self.pending_jump_from = None;
                    engine.seek_end(action == KeyAction::End)?
                };
            }
            KeyAction::FocusNext => outcome.redraw |= self.cycle_focus(engine, true)?,
            KeyAction::FocusPrevious => outcome.redraw |= self.cycle_focus(engine, false)?,
            KeyAction::ToggleSidebar => outcome.redraw = self.toggle_sidebar(engine)?,
            KeyAction::Activate => {
                if self.focus == FocusTarget::Sidebar {
                    let from = engine.navigation_position();
                    let moved = engine.activate_sidebar_selection()?;
                    if moved {
                        self.record_history_jump(from, engine.navigation_position());
                    } else if engine.runtime().has_pending_jump() {
                        self.pending_jump_from = from;
                    }
                    outcome.redraw = moved;
                } else {
                    outcome.activated = Some(self.focus);
                    if self.focus == FocusTarget::Link
                        && engine.runtime_mut().activate_focused_fold()
                    {
                        engine.request_render();
                        outcome.redraw = true;
                    } else if self.focus == FocusTarget::Link {
                        outcome.activated_link = engine.link_focus();
                        let from = engine.navigation_position();
                        let activation = engine.activate_focused_link()?;
                        let external = engine.last_external_open();
                        if self.pending_fragment_from.is_none() {
                            self.pending_fragment_from = from;
                        }
                        self.commit_fragment(engine);
                        if !matches!(
                            activation.status,
                            crate::fragment::FragmentActivationStatus::BudgetExhausted
                                | crate::fragment::FragmentActivationStatus::PendingIndex
                                | crate::fragment::FragmentActivationStatus::PendingDocument
                        ) {
                            self.pending_fragment_from = None;
                        }
                        outcome.redraw = activation.moved
                            || activation.status.shows_notice()
                            || external.is_some_and(|outcome| outcome.status.shows_notice());
                        outcome.fragment_activation = Some(activation);
                        outcome.external_open = external;
                        self.last_fragment_activation = Some(activation);
                        self.last_external_open = external;
                    }
                }
            }
            KeyAction::HistoryBack => {
                if let Some(target) = self.history.peek_back() {
                    let restored = engine.restore_navigation_position(target)?;
                    if restored.is_ready() && restored.moved() {
                        self.history.commit_back();
                        outcome.redraw = restored.moved();
                    } else if restored == HistoryRestoreStatus::BudgetExhausted {
                        self.pending_history = Some((false, target));
                        engine.request_render();
                        outcome.redraw = true;
                    }
                }
            }
            KeyAction::HistoryForward => {
                if let Some(target) = self.history.peek_forward() {
                    let restored = engine.restore_navigation_position(target)?;
                    if restored.is_ready() && restored.moved() {
                        self.history.commit_forward();
                        outcome.redraw = restored.moved();
                    } else if restored == HistoryRestoreStatus::BudgetExhausted {
                        self.pending_history = Some((true, target));
                        engine.request_render();
                        outcome.redraw = true;
                    }
                }
            }
            KeyAction::None => {}
        }
        if (self.focus != previous_focus || outcome.redraw)
            && matches!(action, KeyAction::FocusNext | KeyAction::FocusPrevious)
        {
            outcome.redraw |= engine.pause_following();
        }
        outcome.redraw |= normalized_redraw || end_cancelled;
        Ok(outcome)
    }

    pub fn resize(
        &mut self,
        engine: &mut StreamEngine,
        rows: u16,
        cols: u16,
    ) -> Result<AppOutcome, StreamError> {
        let resize = engine.resize(rows, cols)?;
        let mut redraw = resize.height_changed || resize.width_changed;
        if self.sidebar_visible {
            let layout = engine.set_sidebar_visible(sidebar_width(cols).is_some())?;
            self.sidebar_visible = layout.visible;
            if !layout.visible {
                redraw |= self.normalize_focus(engine);
            }
            redraw |= layout.changed;
        }
        if redraw {
            engine.request_render();
        }
        Ok(AppOutcome {
            redraw,
            ..AppOutcome::default()
        })
    }

    fn commit_fragment(&mut self, engine: &StreamEngine) {
        let (serial, result) = engine.runtime().fragment_completion();
        if serial == self.fragment_serial {
            return;
        }
        self.fragment_serial = serial;
        if let Some(result) = result {
            self.last_fragment_activation = Some(result);
            let from = self.pending_fragment_from.take();
            let to = result.target.map(|t| HistoryPosition {
                source: t.source,
                row_offset: engine.navigation_position().map_or(0, |p| p.row_offset),
            });
            self.record_history_jump(from, to);
        }
    }
    pub fn click_link(
        &mut self,
        engine: &mut StreamEngine,
        row: u16,
        col: u16,
    ) -> Result<AppOutcome, StreamError> {
        if !engine.runtime_mut().focus_fold_at(row, col)
            && !engine.runtime_mut().focus_link_at(row, col)
        {
            return Ok(AppOutcome::default());
        }
        self.focus = FocusTarget::Link;
        self.handle_key(
            engine,
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        )
    }
    pub fn frame_committed(&mut self, engine: &StreamEngine) {
        self.commit_fragment(engine);
        if !engine.runtime().has_pending_jump()
            && let Some((forward, target)) = self.pending_history.take()
            && engine
                .runtime()
                .history_restore_result()
                .is_some_and(|(requested, status)| {
                    requested == target && status.is_ready() && status.moved()
                })
        {
            if forward {
                self.history.commit_forward();
            } else {
                self.history.commit_back();
            }
        }
        if !engine.runtime().has_pending_jump()
            && let Some(from) = self.pending_jump_from.take()
        {
            self.record_history_jump(Some(from), engine.navigation_position());
        }
    }

    fn scroll_up(&mut self, engine: &mut StreamEngine, rows: u64) -> Result<bool, StreamError> {
        self.pending_jump_from = None;
        let moved = engine.runtime_mut().scroll_up(rows)?;
        if moved {
            self.history.update_current(engine.navigation_position());
            engine.request_render();
        }
        Ok(moved)
    }

    fn scroll_down(&mut self, engine: &mut StreamEngine, rows: u64) -> Result<bool, StreamError> {
        self.pending_jump_from = None;
        let moved = engine.runtime_mut().scroll_down(rows)?;
        if moved {
            self.history.update_current(engine.navigation_position());
            engine.request_render();
        }
        Ok(moved)
    }

    fn record_history_jump(&mut self, from: Option<HistoryPosition>, to: Option<HistoryPosition>) {
        if let Some(to) = to
            && Some(to) != from
        {
            self.history.record_jump(from, to);
        }
    }

    fn move_sidebar_selection(
        &self,
        engine: &mut StreamEngine,
        delta: isize,
    ) -> Result<bool, StreamError> {
        let changed = engine.move_sidebar_selection(delta);
        if changed {
            engine.request_render();
        }
        Ok(changed)
    }

    fn toggle_sidebar(&mut self, engine: &mut StreamEngine) -> Result<bool, StreamError> {
        let layout = engine.set_sidebar_visible(!self.sidebar_visible)?;
        self.sidebar_visible = layout.visible;
        let mut changed = layout.changed;
        if !layout.visible {
            changed |= self.normalize_focus(engine);
        }
        if changed {
            engine.request_render();
        }
        Ok(changed)
    }

    fn normalize_focus(&mut self, engine: &mut StreamEngine) -> bool {
        let mut redraw = false;
        if !self.sidebar_visible && self.focus == FocusTarget::Sidebar {
            self.focus = FocusTarget::Body;
            redraw = true;
        }
        if self.focus == FocusTarget::Link && engine.has_visible_objects() == Some(false) {
            engine.clear_link_focus();
            self.focus = FocusTarget::Body;
            redraw = true;
        }
        redraw
    }

    fn cycle_focus(
        &mut self,
        engine: &mut StreamEngine,
        forward: bool,
    ) -> Result<bool, StreamError> {
        let redraw = self.normalize_focus(engine);
        let links = engine.has_visible_objects();
        if self.focus == FocusTarget::Link {
            return Ok(redraw
                || match links {
                    Some(true) => engine.cycle_object_focus(forward),
                    Some(false) => false,
                    None => false,
                });
        }
        let mut order = vec![FocusTarget::Body];
        if self.sidebar_visible {
            order.push(FocusTarget::Sidebar);
        }
        if links == Some(true) {
            order.push(FocusTarget::Link);
        }
        let current = order
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        for offset in 1..=order.len() {
            let index = if forward {
                (current + offset) % order.len()
            } else {
                (current + order.len() - offset) % order.len()
            };
            let next = order[index];
            if next == self.focus {
                continue;
            }
            if next == FocusTarget::Link {
                if engine.enter_object_focus(forward) {
                    self.focus = FocusTarget::Link;
                    return Ok(true);
                }
                continue;
            }
            self.focus = next;
            return Ok(redraw);
        }
        Ok(redraw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::LinkMode;
    use crate::render::frame::{FrameGeometry, FrameRuntime};

    fn engine() -> StreamEngine {
        StreamEngine::new(
            FrameRuntime::new(
                FrameGeometry {
                    rows: 10,
                    cols: 80,
                    content_width: 80,
                },
                LinkMode::Default,
            )
            .unwrap(),
        )
    }

    #[test]
    fn key_table_covers_navigation_focus_sidebar_history_and_quit() {
        assert_eq!(
            action_for_key(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyAction::Quit
        );
        assert_eq!(
            action_for_key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyAction::Quit
        );
        assert_eq!(
            action_for_key(KeyCode::Up, KeyModifiers::NONE),
            KeyAction::LineUp
        );
        assert_eq!(
            action_for_key(KeyCode::Down, KeyModifiers::NONE),
            KeyAction::LineDown
        );
        assert_eq!(
            action_for_key(KeyCode::PageUp, KeyModifiers::NONE),
            KeyAction::PageUp
        );
        assert_eq!(
            action_for_key(KeyCode::PageDown, KeyModifiers::NONE),
            KeyAction::PageDown
        );
        assert_eq!(
            action_for_key(KeyCode::Home, KeyModifiers::NONE),
            KeyAction::Home
        );
        assert_eq!(
            action_for_key(KeyCode::End, KeyModifiers::NONE),
            KeyAction::End
        );
        assert_eq!(
            action_for_key(KeyCode::Tab, KeyModifiers::NONE),
            KeyAction::FocusNext
        );
        assert_eq!(
            action_for_key(KeyCode::Tab, KeyModifiers::SHIFT),
            KeyAction::FocusPrevious
        );
        assert_eq!(
            action_for_key(KeyCode::BackTab, KeyModifiers::NONE),
            KeyAction::FocusPrevious
        );
        assert_eq!(
            action_for_key(KeyCode::Char('s'), KeyModifiers::NONE),
            KeyAction::ToggleSidebar
        );
        assert_eq!(
            action_for_key(KeyCode::Enter, KeyModifiers::NONE),
            KeyAction::Activate
        );
        assert_eq!(
            action_for_key(KeyCode::Left, KeyModifiers::ALT),
            KeyAction::HistoryBack
        );
        assert_eq!(
            action_for_key(KeyCode::Right, KeyModifiers::ALT),
            KeyAction::HistoryForward
        );
        assert_eq!(
            action_for_key(KeyCode::Char('x'), KeyModifiers::NONE),
            KeyAction::None
        );
    }

    #[test]
    fn sidebar_width_uses_the_p52_placeholder_formula() {
        assert_eq!(sidebar_width(20), None);
        assert_eq!(sidebar_width(39), None);
        assert_eq!(sidebar_width(40), Some(20));
        assert_eq!(sidebar_width(80), Some(24));
        assert_eq!(sidebar_width(120), Some(36));
        assert_eq!(sidebar_width(160), Some(40));
        assert_eq!(content_width(120, false), 120);
        assert_eq!(content_width(120, true), 84);
    }

    #[test]
    fn focus_cycle_without_visible_links_stays_on_focusable_targets() {
        let mut engine = engine();
        let mut app = AppState::default();
        app.cycle_focus(&mut engine, true).unwrap();
        assert_eq!(app.focus, FocusTarget::Body);
        app.cycle_focus(&mut engine, false).unwrap();
        assert_eq!(app.focus, FocusTarget::Body);

        app.sidebar_visible = true;
        app.focus = FocusTarget::Body;
        app.cycle_focus(&mut engine, true).unwrap();
        assert_eq!(app.focus, FocusTarget::Sidebar);
        app.cycle_focus(&mut engine, true).unwrap();
        assert_eq!(app.focus, FocusTarget::Body);
        app.cycle_focus(&mut engine, false).unwrap();
        assert_eq!(app.focus, FocusTarget::Sidebar);
    }

    #[test]
    fn empty_history_is_an_explicit_no_op() {
        let mut history = HistoryState::default();
        assert!(history.is_empty());
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), None);
        assert!(history.is_empty());
        history.record_jump(
            None,
            HistoryPosition {
                source: 42,
                row_offset: 0,
            },
        );
        assert_eq!(history.len(), 1);
        assert_eq!(history.last_source(), Some(42));
        assert!(!history.is_empty());
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), None);
    }
}
