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
//! The prompt pane is editable: `enter` or a double-click hands the keyboard
//! to the editor, `esc` or `tab` gives it back, and `e` opens the prompt in
//! `$EDITOR`.
//! Picking colours, editing variants and driving an agent are still later
//! work, and the state in [`app::App`] is arranged to receive them.

pub mod app;
pub mod panes;
pub mod render_worker;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};

use crate::error::{Error, Result};
use crate::preview::{Backend, Preview, Scale};
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
pub fn run(project: Project, backend: Backend, scale: Option<Scale>, verbose: u8) -> Result<()> {
    // Held for the whole session: a warning printed over the alternate screen
    // corrupts it and cannot be scrolled back to.
    let _quiet = crate::logging::suppress();

    let mut terminal = enter()?;

    // After the alternate screen is up and before any event is read: the
    // capability query writes to stdout and reads the reply from stdin, and
    // anything else touching either would eat it.
    let mut preview = Preview::detect(backend);
    if let Some(scale) = scale {
        preview.set_scale(scale);
    }
    let mut app = App::new(project, preview.name());
    app.verbose = verbose;

    let outcome = event_loop(&mut terminal, &mut app, &mut preview);

    // Restored first, and its own failure reported only if nothing worse
    // happened, so the original error is never masked by the cleanup.
    let restored = leave(&mut terminal);
    outcome.and(restored)
}

/// Take the terminal, putting it into the state the workspace needs.
///
/// Separate from [`enter`] because the terminal is also given up and taken
/// back mid-session, when the prompt is handed to `$EDITOR`. One definition of
/// "the state the workspace needs" rather than two that can drift.
fn grab(out: &mut impl io::Write) -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    enable_raw_mode().map_err(io_error)?;
    // Mouse capture takes the terminal's own text selection with it. That is
    // the accepted trade for a clickable interface, and every terminal worth
    // using offers Shift-drag to select through it anyway.
    execute!(out, EnterAlternateScreen, EnableMouseCapture, cursor::Hide).map_err(io_error)
}

/// Give the terminal back, exactly as it was found.
fn release(out: &mut impl io::Write) -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    disable_raw_mode().map_err(io_error)?;
    execute!(out, DisableMouseCapture, LeaveAlternateScreen, cursor::Show).map_err(io_error)
}

/// A terminal over the standard output, which [`grab`] must already have taken.
///
/// Its buffers start empty, so the first frame drawn through it paints every
/// cell rather than a diff against whatever was there before.
fn fresh_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    Terminal::new(CrosstermBackend::new(io::stdout()))
        .map_err(|source| Error::io("the terminal", source))
}

/// Put the terminal into the state the workspace needs.
fn enter() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    grab(&mut io::stdout())?;
    fresh_terminal()
}

/// Put it back.
fn leave(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    release(terminal.backend_mut())?;
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
        let arrived = app.collect_preview();
        app.update_preview();

        // A new image is written *inside* `draw`, and under tmux that write is
        // over a megabyte of passthrough sequences and takes seconds. Nothing
        // can animate during it, because the call that would paint the spinner
        // is the call that is blocked. So one cheap frame is drawn first, with
        // the spinner and whatever was previously on screen, and the image
        // goes out on the frame after it. That is the difference between a
        // workspace that looks wedged and one that looks busy.
        if arrived {
            app.hold_image();
            terminal
                .draw(|frame| ui::draw(frame, app, preview))
                .map_err(io_error)?;
            app.release_image();
        }

        terminal
            .draw(|frame| ui::draw(frame, app, preview))
            .map_err(io_error)?;

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
                Event::Key(key) if key.kind == KeyEventKind::Press => handle(app, key),
                Event::Mouse(mouse) => handle_mouse(app, mouse),
                _ => {}
            }
            if app.should_quit || !event::poll(Duration::ZERO).map_err(io_error)? {
                break;
            }
        }

        // After the drain rather than inside it: the terminal is torn down and
        // rebuilt here, and doing that with events still queued would feed
        // them to whatever `$EDITOR` turns out to be.
        open_editor(terminal, app, preview)?;
    }

    Ok(())
}

/// The editor the user has nominated, and the arguments it came with.
///
/// `$VISUAL` before `$EDITOR`: the former is by definition the one that can
/// use a whole screen, which is the only kind worth handing a terminal to.
fn editor_command() -> Result<Vec<String>> {
    let settings = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|name| std::env::var(name).ok());
    nominated_editor(settings).ok_or(Error::NoEditor)
}

/// The first of `settings` that actually nominates something.
///
/// Split on whitespace, because `EDITOR="emacsclient -nw"` and
/// `EDITOR="code -w"` are entirely ordinary, and treating the whole string as
/// a program name fails with a "not found" that names the arguments too.
///
/// Taken as an argument rather than read here, so it can be tested without the
/// process-wide environment, which the rest of the suite is also running in.
fn nominated_editor(settings: impl IntoIterator<Item = String>) -> Option<Vec<String>> {
    settings
        .into_iter()
        .map(|value| {
            value
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        // A setting that is empty or all whitespace is not a nomination, and
        // treating it as one would try to run a program with no name.
        .find(|words| !words.is_empty())
}

/// Hand the prompt to `$EDITOR` and take the terminal back afterwards.
///
/// Everything that can go wrong here ends up in the status line rather than
/// being returned: the workspace is the only place an unsaved prompt still
/// exists, so an editor that is missing, refuses to start or exits angrily
/// must not close it.
fn open_editor(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    preview: &mut Preview,
) -> Result<()> {
    if !app.take_system_editor_request() {
        return Ok(());
    }

    match edit_externally(&app.draft()) {
        Ok(Some(edited)) => app.set_draft(&edited),
        // The editor exited non-zero, which is how every editor worth the name
        // says the edit was abandoned. The buffer is left exactly as it was.
        Ok(None) => app.notice = Some("editor exited without saving".to_owned()),
        Err(error) => app.notice = Some(format!("editor failed: {error}")),
    }

    // The alternate screen was left, and the terminal dropped the transmitted
    // image with it. Without this the preview comes back empty and stays that
    // way until something unrelated happens to change the render.
    preview.invalidate();

    // A new terminal rather than `Terminal::clear`, which snapshots the cursor
    // position first — and reading that means writing a query and blocking on
    // the reply over the same stdin the event loop reads, which fails outright
    // wherever nothing answers, a pty under test included. A fresh terminal
    // has an empty previous buffer, so the next frame repaints every cell,
    // which is the only thing that was wanted from the clear.
    *terminal = fresh_terminal()?;

    Ok(())
}

/// Run the editor over `text`, returning what came back.
///
/// `None` means the editor exited non-zero and the text should be left alone.
///
/// The terminal is given up for the duration and taken back afterwards
/// whatever happens, including when the editor cannot be started at all: a
/// workspace that returns to a raw-mode terminal with no alternate screen is
/// unusable, and that must not depend on the editor behaving.
fn edit_externally(text: &str) -> Result<Option<String>> {
    let command = editor_command()?;

    // `.md` so the editor reaches for prose mode and spell checking rather
    // than treating a paragraph as source.
    let file = tempfile::Builder::new()
        .prefix("shaipe-prompt-")
        .suffix(".md")
        .tempfile()
        .map_err(|source| Error::io("a temporary file", source))?;
    let path = file.path().to_path_buf();
    std::fs::write(&path, text).map_err(|source| Error::io(&path, source))?;

    release(&mut io::stdout())?;
    let status = std::process::Command::new(&command[0])
        .args(&command[1..])
        .arg(&path)
        .status();
    let regrabbed = grab(&mut io::stdout());

    // The editor is likely to have asked the terminal a question on its way
    // out — vim asks for the background colour — and the reply arrives on
    // stdin once we already have the terminal back. Left there it is read as
    // a keypress, or painted into the workspace as stray escape codes. See
    // https://ratatui.rs/recipes/apps/spawn-vim/.
    drain_events()?;

    regrabbed?;

    let status = status.map_err(|source| Error::io(&command[0], source))?;
    if !status.success() {
        return Ok(None);
    }

    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|source| Error::io(&path, source))
}

/// Throw away everything the terminal has queued up.
fn drain_events() -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    while event::poll(Duration::ZERO).map_err(io_error)? {
        let _ = event::read().map_err(io_error)?;
    }
    Ok(())
}

/// Apply a keypress.
///
/// Separated from the loop so it can be tested without a terminal.
fn handle(app: &mut App, key: KeyEvent) {
    // The editor owns every printable key while it is engaged, so it is asked
    // first. Saving is the exception: the editor's key map leaves `ctrl-s`
    // alone, and a save that only works from outside the editor would be a
    // save nobody reaches for.
    let save = key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL);
    if app.is_editing() && !save {
        app.edit_key(key);
        return;
    }

    // Any key other than another quit withdraws a pending confirmation, so the
    // warning does not sit over an unrelated action.
    if !matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
        app.cancel_quit_confirmation();
    }

    match key.code {
        _ if save => app.save(),
        KeyCode::Char('q') | KeyCode::Esc => app.request_quit(),
        KeyCode::Enter if app.focus == Focus::Prompt => app.engage_editor(),
        // Only on the focused prompt pane, and only while the editor is not
        // engaged — once it is, `e` is a letter someone is writing. The
        // editor's own key map has `ctrl-e` for the end of the line, so there
        // is no chord left inside it that would not be taking something else
        // away.
        KeyCode::Char('e') if app.focus == Focus::Prompt => app.request_system_editor(),
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

    // While editing, the prompt pane's rectangle belongs to the editor. Only
    // the wheel does anything with it — the editor has no way to place the
    // cursor at a screen cell — but a click there must still not be taken as
    // a click on a pane. A click anywhere else is a statement that the prompt
    // is no longer what the user is working on, so the editor is let go of
    // before that click is handled as it normally would be.
    if app.is_editing() {
        if app.pane_at(column, row) == Some(Focus::Prompt) {
            app.edit_mouse(mouse);
            return;
        }
        app.disengage_editor();
    }

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

            // A second click means "act on this", and what that is depends on
            // the pane: a render specification is exported, and the prompt is
            // opened for editing. Double-clicking prose to edit it is what
            // every other text field does.
            if repeat {
                match focus {
                    Focus::Renders => app.export_selected_render(),
                    Focus::Prompt => app.engage_editor(),
                    _ => {}
                }
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

    /// A keypress with no modifiers.
    fn press(app: &mut App, code: KeyCode) {
        handle(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// A keypress with control held.
    fn control(app: &mut App, code: KeyCode) {
        handle(app, KeyEvent::new(code, KeyModifiers::CONTROL));
    }

    /// Type text into an engaged editor.
    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    /// Focus the prompt pane and hand the keyboard to the editor.
    ///
    /// On a project with no prompt, so that what a test types is the whole
    /// buffer rather than the fixture's prose with an insertion in front of
    /// it, and so that the workspace starts out clean.
    fn engaged() -> App {
        let mut project = fixtures::project();
        project.metadata_mut().prompt = None;

        let mut app = App::new(project, "blocks");
        app.focus = Focus::Prompt;
        press(&mut app, KeyCode::Enter);
        assert!(!app.is_dirty(), "engaging the editor is not an edit");
        app
    }

    #[test]
    fn q_and_escape_both_leave_the_workspace() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut app = app();
            press(&mut app, key);
            assert!(app.should_quit, "{key:?} should quit");
        }
    }

    #[test]
    fn tab_cycles_focus_through_every_pane_and_back_to_the_first() {
        let mut app = app();
        let first = app.focus;
        for _ in 0..Focus::COUNT {
            press(&mut app, KeyCode::Tab);
        }
        assert_eq!(app.focus, first);
    }

    #[test]
    fn the_arrow_keys_move_the_selection_within_the_focused_pane() {
        let mut app = app();
        app.focus = Focus::Variants;

        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_variant(), 1);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected_variant(), 0);
    }

    #[test]
    fn enter_on_the_prompt_pane_hands_the_keyboard_to_the_editor() {
        let mut app = app();
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Enter);
        assert!(app.is_editing());
    }

    #[test]
    fn typing_in_the_prompt_editor_changes_the_project_prompt() {
        let mut app = engaged();
        type_text(&mut app, "a bold mark");

        assert!(
            app.project
                .metadata()
                .prompt
                .as_deref()
                .is_some_and(|prompt| prompt.starts_with("a bold mark")),
            "{:?}",
            app.project.metadata().prompt
        );
        assert!(app.is_dirty());
    }

    #[test]
    fn a_key_typed_into_the_editor_is_not_a_workspace_shortcut() {
        // Every letter the workspace binds is also a letter someone writes.
        for character in ['q', 'j', 'k', 'r', 'e'] {
            let mut app = engaged();
            type_text(&mut app, &character.to_string());

            assert!(!app.should_quit, "{character} was typed, not pressed");
            assert!(app.is_editing(), "{character} left the editor");
            assert_eq!(
                app.draft(),
                character.to_string(),
                "{character} did not reach the buffer"
            );
        }
    }

    #[test]
    fn escape_leaves_the_editor_rather_than_quitting_the_workspace() {
        let mut app = engaged();
        type_text(&mut app, "x");

        press(&mut app, KeyCode::Esc);

        assert!(!app.is_editing());
        assert!(!app.should_quit, "leaving the editor is not quitting");
        assert_eq!(app.focus, Focus::Prompt, "and does not move the focus");
    }

    #[test]
    fn tab_while_editing_moves_to_the_next_pane_rather_than_indenting() {
        // Tab is how the whole workspace is navigated, and a literal tab in a
        // paragraph of prose is worth much less than a consistent way out.
        let mut app = engaged();
        type_text(&mut app, "prose");

        press(&mut app, KeyCode::Tab);

        assert!(!app.is_editing());
        assert_eq!(app.focus, Focus::Palette);
        assert_eq!(app.draft(), "prose", "no tab reached the buffer");
    }

    #[test]
    fn opening_and_closing_the_prompt_editor_without_typing_changes_nothing() {
        let mut app = app();
        let before = app.project.metadata().prompt.clone();

        app.focus = Focus::Prompt;
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);

        assert_eq!(app.project.metadata().prompt, before);
        assert!(!app.is_dirty(), "nothing was typed, so nothing is unsaved");
    }

    #[test]
    fn e_on_the_focused_prompt_pane_requests_the_system_editor() {
        let mut app = app();
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Char('e'));
        assert!(app.wants_system_editor());
    }

    #[test]
    fn e_on_another_pane_is_not_a_request_for_the_system_editor() {
        let mut app = app();
        app.focus = Focus::Variants;

        press(&mut app, KeyCode::Char('e'));
        assert!(!app.wants_system_editor());
    }

    #[test]
    fn quitting_with_unsaved_changes_asks_before_discarding_them() {
        let mut app = engaged();
        type_text(&mut app, "unsaved");
        press(&mut app, KeyCode::Esc);

        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit, "unsaved work is worth one question");
        assert!(app.wants_quit_confirmation());

        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit, "and no more than one");
    }

    #[test]
    fn an_unedited_workspace_quits_without_asking() {
        let mut app = app();
        press(&mut app, KeyCode::Char('q'));

        assert!(app.should_quit);
        assert!(!app.wants_quit_confirmation());
    }

    #[test]
    fn anything_but_another_quit_withdraws_the_unsaved_changes_question() {
        let mut app = engaged();
        type_text(&mut app, "unsaved");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('q'));

        press(&mut app, KeyCode::Char('j'));
        assert!(!app.wants_quit_confirmation());

        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit, "the question is asked again");
    }

    #[test]
    fn control_s_saves_even_from_inside_the_editor() {
        // A real file, because the shared fixture's path is a bare
        // `logo.svg` — saving that from a test would rewrite the repository's
        // own artwork.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).expect("the fixture is writable");
        let project = crate::project::Project::open(&path).expect("the fixture is a project");

        let mut app = App::new(project, "blocks");
        app.focus = Focus::Prompt;
        press(&mut app, KeyCode::Enter);
        app.set_draft("");
        type_text(&mut app, "saved from the editor");

        control(&mut app, KeyCode::Char('s'));

        assert!(app.is_editing(), "saving does not leave the editor");
        assert!(!app.is_dirty(), "the file now holds what the buffer does");
        assert_eq!(app.draft(), "saved from the editor", "no s was inserted");
        assert!(
            std::fs::read_to_string(&path)
                .expect("the project was written")
                .contains("saved from the editor"),
            "ctrl-s reached the workspace rather than being typed"
        );
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
    fn double_clicking_the_prompt_opens_it_for_editing() {
        let mut app = laid_out();
        let prompt = app.area(Focus::Prompt);
        let (column, row) = (prompt.x + 2, prompt.y + 2);

        click(&mut app, column, row);
        assert!(!app.is_editing(), "one click only focuses");
        assert_eq!(app.focus, Focus::Prompt);

        click(&mut app, column, row);
        assert!(app.is_editing());
    }

    #[test]
    fn double_clicking_a_pane_that_has_nothing_to_open_does_nothing() {
        let mut app = laid_out();
        let palette = app.area(Focus::Palette);

        click(&mut app, palette.x + 2, palette.y + 2);
        click(&mut app, palette.x + 2, palette.y + 2);

        assert!(!app.is_editing());
        assert!(app.notice.is_none());
    }

    #[test]
    fn the_editor_command_prefers_visual_and_keeps_the_arguments_it_came_with() {
        // `EDITOR="emacsclient -nw"` is entirely ordinary, and treating the
        // whole string as a program name fails with a "not found" that names
        // the arguments too.
        let resolve =
            |settings: &[&str]| nominated_editor(settings.iter().map(|value| (*value).to_owned()));

        assert_eq!(
            resolve(&["emacsclient -nw", "vi"]),
            Some(vec!["emacsclient".to_owned(), "-nw".to_owned()])
        );
        assert_eq!(resolve(&["vi"]), Some(vec!["vi".to_owned()]));
        // An empty or blank setting is not a nomination.
        assert_eq!(resolve(&["  ", "vi"]), Some(vec!["vi".to_owned()]));
        assert_eq!(resolve(&[]), None);
        assert_eq!(resolve(&[""]), None);
    }

    #[test]
    fn a_workspace_with_no_editor_configured_says_what_to_set() {
        // On the status line, which is `Display` and nothing more: the advice
        // has to be in the message itself, because the `help` is never
        // rendered on this path.
        let error = Error::NoEditor;
        let message = error.to_string();

        assert!(message.contains("$VISUAL"), "{message}");
        assert!(message.contains("$EDITOR"), "{message}");
        assert_eq!(
            miette::Diagnostic::code(&error).map(|code| code.to_string()),
            Some("shaipe::tui::no_editor".to_owned())
        );
    }

    #[test]
    fn a_key_with_no_binding_changes_nothing() {
        let mut app = app();
        let before = (app.focus, app.selected_variant(), app.should_quit);
        handle(
            &mut app,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
        );
        assert_eq!((app.focus, app.selected_variant(), app.should_quit), before);
    }
}
