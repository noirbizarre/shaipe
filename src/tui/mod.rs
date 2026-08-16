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
pub mod render_worker;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};

use crate::error::{Error, Result};
use crate::preview::{Backend, Preview};
use crate::project::Project;

use app::{App, Focus};

/// How long to wait for a key when nothing is happening.
///
/// The workspace has nothing that changes on its own, so this only bounds how
/// long a resize goes unnoticed. Long enough not to spin a CPU, short enough
/// not to feel stuck.
const IDLE_TICK: Duration = Duration::from_millis(250);

/// How long to wait while a render is in flight.
///
/// Short enough for the spinner to animate and for a finished render to appear
/// promptly. Only paid while something is actually happening.
const BUSY_TICK: Duration = Duration::from_millis(80);

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
pub fn run(project: Project, backend: Backend, verbose: u8) -> Result<()> {
    // Held for the whole session: a warning printed over the alternate screen
    // corrupts it and cannot be scrolled back to.
    let _quiet = crate::logging::suppress();

    let mut terminal = enter()?;

    // After the alternate screen is up and before any event is read: the
    // capability query writes to stdout and reads the reply from stdin, and
    // anything else touching either would eat it.
    let mut preview = Preview::detect(backend);
    let mut app = App::new(project, preview.name());
    app.verbose = verbose;

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
    // Mouse capture takes the terminal's own text selection with it. That is
    // the accepted trade for a clickable interface, and every terminal worth
    // using offers Shift-drag to select through it anyway.
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide
    )
    .map_err(io_error)?;
    Terminal::new(CrosstermBackend::new(stdout)).map_err(io_error)
}

/// Put it back.
fn leave(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    disable_raw_mode().map_err(io_error)?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::Show
    )
    .map_err(io_error)?;
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
        // Drawn *before* the first render is asked for. That is what tells the
        // application how large the preview pane is, so the first render is
        // made once at the right size instead of once at a guessed 512 and
        // again at the real one — two rasterises and, under Kitty, two
        // megabyte-scale transmissions before anything appeared on screen.
        terminal
            .draw(|frame| ui::draw(frame, app, preview))
            .map_err(io_error)?;

        app.collect_preview();
        app.update_preview();

        let tick = if app.is_rendering() {
            BUSY_TICK
        } else {
            IDLE_TICK
        };
        if !event::poll(tick).map_err(io_error)? {
            continue;
        }

        // Every event that has already arrived is applied before the next
        // frame. Without this, a burst of keys draws — and under Kitty
        // transmits — one image per key, and nobody sees the intermediate
        // ones.
        loop {
            match event::read().map_err(io_error)? {
                // Windows reports press *and* release; acting on both would
                // move every selection two steps at a time.
                Event::Key(key) if key.kind == KeyEventKind::Press => handle(app, key.code),
                Event::Mouse(mouse) => handle_mouse(app, mouse),
                _ => {}
            }
            if app.should_quit || !event::poll(Duration::ZERO).map_err(io_error)? {
                break;
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

/// Apply a mouse event.
///
/// Separated from the loop for the same reason as [`handle`]: none of this
/// needs a terminal, and all of it is easy to get subtly wrong.
fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    let (column, row) = (mouse.column, mouse.row);

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // The divider is checked first: it sits on the preview pane's left
            // edge, so a click there would otherwise be swallowed as a click
            // on the preview.
            if column == app.column_width || column + 1 == app.column_width {
                app.begin_resize();
                return;
            }

            let Some(focus) = app.pane_at(column, row) else {
                return;
            };
            let repeat = app.register_click(focus, row);
            app.focus = focus;
            app.select_at(focus, row);

            if repeat && focus == Focus::Renders {
                app.export_selected_render();
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.is_resizing() {
                app.resize_column(column, app.total_width());
            }
        }
        MouseEventKind::Up(MouseButton::Left) => app.end_resize(),
        // The pane under the pointer scrolls, not the focused one: reaching
        // for the wheel over a list is an unambiguous statement about which
        // list is meant.
        MouseEventKind::ScrollDown => {
            if let Some(focus) = app.pane_at(column, row) {
                app.select_next_in(focus);
            }
        }
        MouseEventKind::ScrollUp => {
            if let Some(focus) = app.pane_at(column, row) {
                app.select_previous_in(focus);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;

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

    /// A left-button press at a point.
    fn click(app: &mut App, column: u16, row: u16) {
        handle_mouse(
            app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            },
        );
    }

    /// A wheel event at a point.
    fn wheel(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
        handle_mouse(
            app,
            MouseEvent {
                kind,
                column,
                row,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            },
        );
    }

    /// Lay the panes out, which is what gives them the areas clicks hit.
    fn laid_out() -> App {
        let mut app = app();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        let mut preview = Preview::detect(Backend::Blocks);
        app.refresh_preview();
        terminal
            .draw(|frame| ui::draw(frame, &mut app, &mut preview))
            .unwrap();
        app
    }

    #[test]
    fn clicking_a_pane_focuses_it() {
        let mut app = laid_out();
        app.focus = Focus::Prompt;

        let renders = app.area(Focus::Renders);
        click(&mut app, renders.x + 2, renders.y + 1);
        assert_eq!(app.focus, Focus::Renders);
    }

    #[test]
    fn clicking_a_row_selects_it() {
        let mut app = laid_out();
        let variants = app.area(Focus::Variants);

        // The second row of content: one row of border, then one entry.
        click(&mut app, variants.x + 2, variants.y + 2);
        assert_eq!(app.selected_variant(), 1);
    }

    #[test]
    fn clicking_a_panes_border_focuses_it_without_moving_the_selection() {
        let mut app = laid_out();
        let variants = app.area(Focus::Variants);
        app.focus = Focus::Variants;
        app.select_next();
        let before = app.selected_variant();

        click(&mut app, variants.x + 2, variants.y);
        assert_eq!(app.focus, Focus::Variants);
        assert_eq!(app.selected_variant(), before);
    }

    #[test]
    fn the_wheel_scrolls_the_pane_under_the_pointer_not_the_focused_one() {
        // Reaching for the wheel over a list is an unambiguous statement about
        // which list is meant, and it must not require clicking first.
        let mut app = laid_out();
        app.focus = Focus::Prompt;
        let variants = app.area(Focus::Variants);

        wheel(
            &mut app,
            MouseEventKind::ScrollDown,
            variants.x + 2,
            variants.y + 1,
        );
        assert_eq!(app.focus, Focus::Prompt, "scrolling must not steal focus");
        assert_eq!(app.selection(Focus::Variants), 1);

        wheel(
            &mut app,
            MouseEventKind::ScrollUp,
            variants.x + 2,
            variants.y + 1,
        );
        assert_eq!(app.selection(Focus::Variants), 0);
    }

    #[test]
    fn dragging_the_divider_resizes_the_description_column() {
        let mut app = laid_out();
        let divider = app.column_width;

        click(&mut app, divider, 5);
        assert!(app.is_resizing());

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 50,
                row: 5,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            },
        );
        assert_eq!(app.column_width, 50);

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 50,
                row: 5,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            },
        );
        assert!(!app.is_resizing());
    }

    #[test]
    fn the_column_cannot_be_dragged_narrow_enough_to_hide_either_side() {
        let mut app = laid_out();
        let width = app.total_width();

        app.resize_column(0, width);
        assert!(app.column_width >= app::MIN_COLUMN);

        app.resize_column(width, width);
        assert!(width - app.column_width >= app::MIN_PREVIEW);
    }

    #[test]
    fn two_clicks_in_the_same_place_are_a_double_click_and_two_elsewhere_are_not() {
        let mut app = laid_out();

        assert!(
            !app.register_click(Focus::Renders, 4),
            "the first click is never a repeat"
        );
        assert!(app.register_click(Focus::Renders, 4), "the second is");
        // Cleared afterwards, so a triple-click is one double-click and then a
        // fresh gesture, rather than firing the action twice.
        assert!(
            !app.register_click(Focus::Renders, 4),
            "the third starts again"
        );
        assert!(
            !app.register_click(Focus::Renders, 9),
            "a different row is a new click, however fast"
        );
    }

    #[test]
    fn double_clicking_a_render_specification_writes_it() {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(directory.path()).unwrap();

        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Renders;
        app.export_selected_render();
        let notice = app.notice.clone().unwrap_or_default();

        std::env::set_current_dir(previous).unwrap();

        assert!(notice.contains("favicon-32.png"), "{notice}");
        assert!(directory.path().join("dist/favicon-32.png").is_file());
    }

    #[test]
    fn a_key_with_no_binding_changes_nothing() {
        let mut app = app();
        let before = (app.focus, app.selected_variant(), app.should_quit);
        handle(&mut app, KeyCode::Char('z'));
        assert_eq!((app.focus, app.selected_variant(), app.should_quit), before);
    }
}
