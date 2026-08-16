//! The interactive workspace.
//!
//! ```text
//! ┌───────────────────────┬───────────────────────────┐
//! │ prompt                │                           │
//! │ palette               │        preview            │
//! │ variants              │                           │
//! │ render specs          │                           │
//! └───────────────────────┴───────────────────────────┘
//! ```
//!
//! The left column describes the project, the right shows what the selection
//! renders to. The preview goes through [`crate::preview`], so the TUI knows
//! nothing about escape sequences or which graphics protocol is in use, and
//! it renders through [`crate::render`], so it shows exactly what
//! `shaipe render` would write — not an approximation of it.
//!
//! There is no editor here yet. The prompt pane displays and scrolls; editing
//! text, picking colours and driving an agent are all later work, and the
//! state in [`app::App`] is arranged to receive them.

pub mod app;
pub mod panes;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};

use crate::error::{Error, Result};
use crate::preview::{Backend, Preview};
use crate::project::Project;

use app::App;

/// How long to wait for a key before redrawing anyway.
///
/// The workspace has nothing that changes on its own, so this only bounds how
/// long a resize goes unnoticed. Long enough not to spin a CPU, short enough
/// not to feel stuck.
const TICK: Duration = Duration::from_millis(250);

/// Open a project in the interactive workspace.
///
/// # Errors
///
/// Returns [`Error::Io`] if the terminal cannot be put into or taken out of
/// raw mode, and whatever rendering a preview returns.
///
/// The terminal is restored before any error is returned. A command that
/// leaves a terminal in raw mode with the alternate screen active is worse
/// than one that simply fails.
pub fn run(project: Project, backend: Backend) -> Result<()> {
    // Held for the whole session: a warning printed over the alternate screen
    // corrupts it and cannot be scrolled back to.
    let _quiet = crate::logging::suppress();

    let mut terminal = enter()?;

    // After the alternate screen is up and before any event is read: the
    // capability query writes to stdout and reads the reply from stdin, and
    // anything else touching either would eat it.
    let mut preview = Preview::detect(backend);
    let mut app = App::new(project, preview.name());

    let outcome = event_loop(&mut terminal, &mut app, &mut preview);

    // Restored first, and its own failure reported only if nothing worse
    // happened, so the original error is never masked by the cleanup.
    let restored = leave(&mut terminal);
    outcome.and(restored)
}

/// Put the terminal into the state the workspace needs.
fn enter() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    let io_error = |source| Error::io("the terminal", source);

    enable_raw_mode().map_err(io_error)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide).map_err(io_error)?;
    Terminal::new(CrosstermBackend::new(stdout)).map_err(io_error)
}

/// Put it back.
fn leave(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    disable_raw_mode().map_err(io_error)?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show).map_err(io_error)?;
    terminal.show_cursor().map_err(io_error)
}

/// Draw, wait for input, repeat.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    preview: &mut Preview,
) -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    while !app.should_quit {
        app.refresh_preview();

        terminal
            .draw(|frame| ui::draw(frame, app, preview))
            .map_err(io_error)?;

        if !event::poll(TICK).map_err(io_error)? {
            continue;
        }
        if let Event::Key(key) = event::read().map_err(io_error)? {
            // Windows reports press *and* release; acting on both would move
            // every selection two steps at a time.
            if key.kind == KeyEventKind::Press {
                handle(app, key.code);
            }
        }
    }

    Ok(())
}

/// Apply a keypress.
///
/// Separated from the loop so it can be tested without a terminal.
fn handle(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Tab | KeyCode::Char('j') => app.focus_next(),
        KeyCode::BackTab | KeyCode::Char('k') => app.focus_previous(),
        KeyCode::Down => app.select_next(),
        KeyCode::Up => app.select_previous(),
        KeyCode::Char('r') => app.invalidate_preview(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;
    use app::Focus;

    fn app() -> App {
        App::new(fixtures::project(), "blocks")
    }

    #[test]
    fn q_and_escape_both_leave_the_workspace() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut app = app();
            handle(&mut app, key);
            assert!(app.should_quit, "{key:?} should quit");
        }
    }

    #[test]
    fn tab_cycles_focus_through_every_pane_and_back_to_the_first() {
        let mut app = app();
        let first = app.focus;
        for _ in 0..Focus::COUNT {
            handle(&mut app, KeyCode::Tab);
        }
        assert_eq!(app.focus, first);
    }

    #[test]
    fn the_arrow_keys_move_the_selection_within_the_focused_pane() {
        let mut app = app();
        app.focus = Focus::Variants;

        handle(&mut app, KeyCode::Down);
        assert_eq!(app.selected_variant(), 1);
        handle(&mut app, KeyCode::Up);
        assert_eq!(app.selected_variant(), 0);
    }

    #[test]
    fn a_key_with_no_binding_changes_nothing() {
        let mut app = app();
        let before = (app.focus, app.selected_variant(), app.should_quit);
        handle(&mut app, KeyCode::Char('z'));
        assert_eq!((app.focus, app.selected_variant(), app.should_quit), before);
    }
}
