use std::io;
use std::panic;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::cli::LinkMode;

#[cfg(debug_assertions)]
const PANIC_TEST_ENV: &str = "MDTUI_TEST_PANIC_AFTER_ENTER";

pub fn run(_link_mode: LinkMode) -> io::Result<()> {
    let restore = RestoreState::new();
    install_panic_hook(restore.clone());
    enable_raw_mode()?;
    restore.activate();
    if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, Hide) {
        restore.restore();
        return Err(error);
    }
    let _session = TerminalSession { restore };

    maybe_panic_for_test();

    loop {
        match event::read()? {
            Event::Key(key)
                if (key.is_press() || key.is_repeat()) && should_exit(key.code, key.modifiers) =>
            {
                return Ok(());
            }
            _ => {}
        }
    }
}

fn should_exit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Char('q'))
        || (matches!(code, KeyCode::Char('c')) && modifiers.contains(KeyModifiers::CONTROL))
}

#[derive(Clone)]
struct RestoreState {
    active: Arc<AtomicBool>,
}

impl RestoreState {
    fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
        }
    }

    fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }

    fn restore(&self) {
        if self.active.swap(false, Ordering::SeqCst) {
            restore_terminal();
        }
    }
}

struct TerminalSession {
    restore: RestoreState,
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore.restore();
    }
}

fn install_panic_hook(restore: RestoreState) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore.restore();
        previous(info);
    }));
}

fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

#[cfg(debug_assertions)]
fn maybe_panic_for_test() {
    if std::env::var_os(PANIC_TEST_ENV).is_some() {
        panic!("P02 panic recovery self-check");
    }
}

#[cfg(not(debug_assertions))]
fn maybe_panic_for_test() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_quit_keys() {
        assert!(should_exit(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(should_exit(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!should_exit(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(!should_exit(KeyCode::Char('x'), KeyModifiers::NONE));
    }
}
