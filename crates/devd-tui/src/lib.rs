//! The `devd` terminal dashboard.
//!
//! # The same registry, a fourth time
//!
//! This crate owns no business logic either. It is a client of the capability
//! registry: every panel is filled by dispatching a capability, and every action
//! is a capability call. A destructive action shows the plan the registry
//! produced and runs only after the person at the keyboard says yes, which is the
//! same two-step decision the CLI and the application window use.
//!
//! # Why a [`Backend`] trait
//!
//! A terminal dashboard is the least testable kind of program if it is written
//! as one. Here the whole thing is split so that nothing that matters needs a
//! terminal:
//!
//! * [`state::State`] is plain data with the row types parsed from the JSON a
//!   capability returns.
//! * [`controller::Controller`] turns a keystroke into capability calls, against
//!   a [`controller::Backend`] trait.
//! * [`view::render`] draws a `State` into a `ratatui` frame.
//! * [`run`] is the only part that owns a terminal, and it is deliberately the
//!   thinnest part.
//!
//! The tests therefore drive keystrokes through a fake backend and assert on
//! state and on the rendered buffer, which is the difference between a dashboard
//! that is believed to work and one that is shown to.

pub mod controller;
pub mod state;
pub mod view;

pub use controller::{Backend, Controller, ExtraChecks, RegistryBackend};
pub use state::{
    AuditRow, BranchRow, Bucket, CapRow, ChangeItem, CheckRow, CommitRow, Key, Overlay, Pending,
    PromptKind, RepoRow, StatusData, View,
};

use std::io;
use std::io::IsTerminal;
use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use devd_core::capability::{Actor, Ctx, Surface};
use devd_core::{Registry, Result};

/// Run the dashboard until the user quits.
///
/// Takes the registry and the invocation context, exactly as `devd ui` does, so
/// the same process can offer either surface over the same core.
pub fn run(registry: Arc<Registry>, mut ctx: Ctx, extra_checks: Vec<ExtraChecks>) -> Result<()> {
    // Refuse before touching the terminal. Entering raw mode on a stdout that is
    // a pipe does not degrade gracefully on Windows: there is no console handle
    // to switch, and crossterm's call into it takes the process down. A clear
    // sentence is worth more than a crash with no output.
    terminal_guard(
        io::stdin().is_terminal(),
        io::stdout().is_terminal(),
    )?;

    // The TUI is a person at a keyboard, and it can answer a prompt. Both the
    // actor kind and the surface matter: `require_interactive` reads the
    // surface, and the tier checks read the kind.
    ctx.actor = Actor::human(Surface::Tui);

    let backend = RegistryBackend::new(registry, ctx, extra_checks);
    let mut controller = Controller::new(Box::new(backend));
    controller.refresh();

    TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .map_err(|e| devd_core::DevdError::internal(format!("cannot open the terminal: {e}")))?;
    let outcome = event_loop(&mut terminal, &mut controller);
    let _restored = TerminalGuard::leave();
    outcome
}

/// Refuse to start unless both ends are a terminal.
///
/// Separated from [`run`] and expressed in booleans so that it can be tested:
/// the harness that runs the tests has no terminal either, so asking the real
/// handles would make the test assert on the harness rather than on the rule.
fn terminal_guard(stdin_is_tty: bool, stdout_is_tty: bool) -> Result<()> {
    if stdin_is_tty && stdout_is_tty {
        return Ok(());
    }
    Err(devd_core::DevdError::unsupported(
        "the dashboard needs an interactive terminal \
         (use `devd ui` for the window, `devd serve` for the API, \
         or `devd capabilities` for the list)",
    ))
}

/// Restores the terminal on every exit path, including a panic-free early return.
struct TerminalGuard;impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()
            .map_err(|e| devd_core::DevdError::internal(format!("cannot enter raw mode: {e}")))?;
        execute!(io::stdout(), EnterAlternateScreen).map_err(|e| {
            devd_core::DevdError::internal(format!("cannot enter the alternate screen: {e}"))
        })?;
        Ok(Self)
    }

    fn leave() -> Result<()> {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        Ok(())
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    controller: &mut Controller,
) -> Result<()> {
    loop {
        terminal
            .draw(|frame| view::render(frame, &controller.state))
            .map_err(|e| devd_core::DevdError::internal(format!("cannot draw: {e}")))?;

        if !event::poll(std::time::Duration::from_millis(250))
            .map_err(|e| devd_core::DevdError::internal(format!("cannot poll: {e}")))?
        {
            continue;
        }
        let event = event::read()
            .map_err(|e| devd_core::DevdError::internal(format!("cannot read input: {e}")))?;

        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(key) = translate(key) {
                    if !controller.handle_key(key) {
                        return Ok(());
                    }
                }
            }
            // A resize just redraws; the next loop iteration does that.
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

/// Map a crossterm key to the small key type the controller speaks.
///
/// Keeping the mapping here is what allows the controller's tests to send `Key`
/// values with no terminal, no event queue and no timing.
fn translate(key: KeyEvent) -> Option<Key> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(c) if ctrl => Some(Key::Ctrl(c.to_ascii_lowercase())),
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::Backspace => Some(Key::Backspace),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn the_terminal_guard_refuses_a_pipe() {
        // What the crash looked like: entering raw mode on a stdout that is a
        // pipe killed the process with no output at all. It must be a sentence.
        let error = terminal_guard(false, true).expect_err("a piped stdin must be refused");
        assert_eq!(error.code(), "unsupported");
        assert!(error.to_string().contains("interactive terminal"));
        assert!(error.to_string().contains("devd ui"), "the error must say what to use instead");

        assert!(terminal_guard(true, false).is_err());
        assert!(terminal_guard(false, false).is_err());
        assert!(terminal_guard(true, true).is_ok());
    }

    #[test]
    fn every_key_the_dashboard_uses_has_a_translation() {
        let cases = [
            (KeyCode::Char('q'), KeyModifiers::NONE, Key::Char('q')),
            (KeyCode::Char('Q'), KeyModifiers::SHIFT, Key::Char('Q')),
            (KeyCode::Char('c'), KeyModifiers::CONTROL, Key::Ctrl('c')),
            // Upper-case control chords are folded, so Ctrl+Shift+C is still
            // Ctrl+C and not a key the dashboard has never heard of.
            (KeyCode::Char('C'), KeyModifiers::CONTROL, Key::Ctrl('c')),
            (KeyCode::Enter, KeyModifiers::NONE, Key::Enter),
            (KeyCode::Esc, KeyModifiers::NONE, Key::Esc),
            (KeyCode::Backspace, KeyModifiers::NONE, Key::Backspace),
            (KeyCode::Up, KeyModifiers::NONE, Key::Up),
            (KeyCode::Down, KeyModifiers::NONE, Key::Down),
            (KeyCode::Tab, KeyModifiers::NONE, Key::Tab),
            (KeyCode::BackTab, KeyModifiers::SHIFT, Key::BackTab),
            (KeyCode::PageUp, KeyModifiers::NONE, Key::PageUp),
            (KeyCode::PageDown, KeyModifiers::NONE, Key::PageDown),
            (KeyCode::Home, KeyModifiers::NONE, Key::Home),
            (KeyCode::End, KeyModifiers::NONE, Key::End),
        ];
        for (code, modifiers, expected) in cases {
            assert_eq!(
                translate(event(code, modifiers)),
                Some(expected),
                "{code:?} with {modifiers:?}"
            );
        }
    }

    #[test]
    fn an_unmapped_key_is_ignored_rather_than_guessed_at() {
        assert_eq!(translate(event(KeyCode::F(5), KeyModifiers::NONE)), None);
        assert_eq!(translate(event(KeyCode::Delete, KeyModifiers::NONE)), None);
    }
}
