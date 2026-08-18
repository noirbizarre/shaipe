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
pub mod transcript;
mod ui;
pub mod watch;

use std::io::{self, Stdout};
use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};
use tokio::time::MissedTickBehavior;

use tokio::sync::mpsc;

use crate::acp::{Agent, AgentChoice, AgentConfig, AgentUpdate, McpServerSpec};
use crate::error::{Error, Result};
use crate::mcp::Listener;
use crate::preview::{Backend, Preview, Scale};
use crate::project::Project;

use crate::tools::{self, Registry, SessionCommand, SessionHandle};

use app::{AgentRequest, AgentStatus, App, Focus};

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
pub async fn run(
    project: Project,
    backend: Backend,
    scale: Option<Scale>,
    verbose: u8,
    agent: AgentChoice,
) -> Result<()> {
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

    // A session the agent can reach, and the agent itself. Neither is fatal:
    // an agent that will not start must not stop someone from reading their
    // own project, so the reason goes in the prompt pane and the workspace
    // opens anyway. Same rule as a preview backend that will not start.
    let (session, commands) = SessionHandle::channel();
    let mut listener = None;
    let mut updates = None;
    let mut connected = None;

    match agent {
        AgentChoice::None => {
            app.agent = AgentStatus::Absent {
                reason: "started without one (--no-agent)".to_owned(),
            };
        }
        // The whole diagnostic, where it will actually be read.
        AgentChoice::Unavailable(reason) => app.agent = AgentStatus::Absent { reason },
        AgentChoice::Start(config) => {
            // Said once, at the start. Shaipe reaches into the agent's own
            // configuration to keep it out of the project file, and doing that
            // without saying so would be worse than not doing it.
            match &config.note {
                Some(note) => app.transcript.push_notice(note.clone()),
                None => app.transcript.push_notice(
                    "the agent may change this project through Shaipe's tools, \
                     and may not edit files or run commands",
                ),
            }

            match start_agent(*config, session).await {
                Ok((agent, socket, stream)) => {
                    // Not `Ready`: the handshake is still going. The prompt says
                    // so, and `AgentUpdate::Ready` is what changes it.
                    app.agent = AgentStatus::Connecting;
                    connected = Some(agent);
                    listener = Some(socket);
                    updates = Some(stream);
                }
                Err(error) => {
                    app.agent = AgentStatus::Absent {
                        reason: error.to_string(),
                    }
                }
            }
        }
    }

    let outcome = event_loop(
        &mut terminal,
        &mut app,
        &mut preview,
        commands,
        updates,
        connected.as_ref(),
    )
    .await;

    // Dropped before the terminal is restored, so the socket goes away and any
    // bridge still connected to it stops rather than lingering.
    drop(listener);
    drop(connected);

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

/// Draw, wait for something to happen, repeat.
///
/// Asynchronous because the workspace waits on more than one thing: the
/// terminal, and — once an agent is wired in — a stream of updates from it and
/// a channel of tool calls coming back the other way. A poll loop over several
/// sources is always either burning a CPU or late.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    preview: &mut Preview,
    mut commands: mpsc::Receiver<SessionCommand>,
    updates: Option<mpsc::Receiver<AgentUpdate>>,
    agent: Option<&Agent>,
) -> Result<()> {
    let io_error = |source| Error::io("the terminal", source);

    // The tools the agent may call. Built once: the set does not change for
    // the life of the workspace.
    let registry = Registry::new();

    // A receiver that never yields, when there is no agent. Cleaner than an
    // `Option` in the `select!`, which would need a branch that is disabled
    // rather than merely empty.
    //
    // The sender is *kept*, and that is the whole point. A receiver whose
    // senders have all been dropped does not block — it returns `None`
    // immediately, forever. Sitting where it does in a `biased` select, that
    // spun the workspace at a full core and starved the tick underneath it, so
    // nothing that depends on the tick ever ran.
    let (_agentless, idle) = mpsc::channel(1);
    let mut updates = updates.unwrap_or(idle);

    // Constructed here rather than in `run`, and the placement is
    // load-bearing: `EventStream` spawns a reader on standard input, and one
    // running during `Preview::detect` would consume the terminal's reply to
    // the capability query. Nothing would fail; every preview would silently
    // become half-blocks on a terminal that supports Kitty. That is the
    // stdout-lock bug from the other direction, and
    // `scripts/check-workspace-detection.py` is what keeps this honest.
    //
    // Rebound rather than borrowed because it has to be *dropped* around
    // `$EDITOR` — see below.
    let mut events = EventStream::new();

    // The render worker is a plain thread with a plain channel, drained on
    // this tick. Rasterising is CPU-bound and belongs on a thread rather than
    // on a runtime worker, and the tick is already the latency budget the
    // spinner animates at.
    let mut ticks = tokio::time::interval(IDLE_TICK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut ticking = IDLE_TICK;

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

        // The tick only needs to be quick while something is animating, and
        // rebuilding the interval every iteration would reset its phase.
        let wanted = if app.is_rendering() {
            BUSY_TICK
        } else {
            IDLE_TICK
        };
        if wanted != ticking {
            ticks = tokio::time::interval(wanted);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
            ticking = wanted;
        }

        tokio::select! {
            // Biased, so terminal input is never starved by anything else in
            // this loop. A workspace that will not answer `q` is broken
            // however good the rest of it is.
            biased;

            event = events.next() => {
                let Some(event) = event else {
                    // Standard input closed. Nothing more can arrive, and
                    // spinning on a finished stream would peg a core.
                    return Ok(());
                };
                apply(app, event.map_err(io_error)?);

                // Every event that has already arrived is applied before the
                // next frame. Without this, a burst of keys draws — and under
                // Kitty transmits — one image per key, and nobody sees the
                // intermediate ones.
                while !app.should_quit
                    && let Some(Some(Ok(event))) = events.next().now_or_never()
                {
                    apply(app, event);
                }
            }

            command = commands.recv() => {
                let Some(command) = command else { continue };

                // The agent reaching back into the workspace. Serviced here,
                // between frames, which is the only moment mutating the
                // project is safe — and the reason there is no lock anywhere
                // in the library. See ADR 009.
                let applied = tools::session::serve(command, &registry, &mut app.project);

                if applied.mutated {
                    // How the preview follows the agent's edit: the same path
                    // `r` takes, so there is one way to do it and not two.
                    app.invalidate_preview();
                    app.mark_dirty();
                    // The prompt may have changed with it, and the editor's
                    // buffer is a copy.
                    app.reload_prompt();
                }
            }

            update = updates.recv() => {
                let Some(update) = update else { continue };

                // What the update says about the agent, before it is applied.
                let said = match &update {
                    AgentUpdate::Ready => Some(AgentStatus::Ready),
                    AgentUpdate::Failed(reason) => Some(AgentStatus::Absent {
                        reason: reason.clone(),
                    }),
                    _ => None,
                };

                // Applied *first*, and that ordering is the whole of it.
                // Deriving the status beforehand read a transcript that was
                // still busy — so `Idle`, the update that ends a turn, latched
                // `Busy` and nothing ever cleared it. The spinner span and the
                // title read "working…" for the rest of the session.
                app.transcript.apply(update);

                app.set_agent(said.unwrap_or(if app.transcript.is_busy() {
                    AgentStatus::Busy
                } else {
                    AgentStatus::Ready
                }));
            }

            _ = ticks.tick() => {
                // Two fields of one `stat`, four times a second at worst. This
                // is what makes an agent that reached for its own editor —
                // which nothing over ACP can prevent — still reach the
                // preview. See ADR 012.
                app.poll_file();
            }
        }

        // Anything a keypress asked the agent for. Done out here rather than
        // in the key handler so that key handling stays synchronous, and
        // testable without a runtime.
        if let Some(request) = app.pending_agent_request.take() {
            dispatch(app, agent, request);
        }

        // After the drain rather than inside it: the terminal is torn down and
        // rebuilt here, and doing that with events still queued would feed
        // them to whatever `$EDITOR` turns out to be.
        //
        // The stream is dropped first and rebuilt after. `EventStream` reads
        // standard input from a thread of its own, and leaving that thread
        // alive while an editor has the terminal means the two compete for
        // every keystroke the user types into `vim`. Dropping it also hands
        // `drain_events` back the blocking API it needs, which cannot see
        // anything the stream has already buffered.
        if app.wants_system_editor() {
            drop(events);
            let outcome = open_editor(terminal, app, preview);
            events = EventStream::new();
            outcome?;
        }
    }

    Ok(())
}

/// Start an agent and the session it will reach back into.
///
/// Returns before the agent has finished its handshake, and that is
/// load-bearing rather than incidental: the agent starts Shaipe's own MCP
/// server during `session/new` and asks it for a tool list before answering,
/// and only this workspace's event loop can answer. Waiting here would
/// deadlock the two and cost the session every one of Shaipe's tools. See
/// [`Agent::start`].
async fn start_agent(
    config: AgentConfig,
    session: SessionHandle,
) -> Result<(Agent, Listener, mpsc::Receiver<AgentUpdate>)> {
    let listener = Listener::bind(session).await?;
    let config = config.with_mcp_server(McpServerSpec::shaipe_bridge(listener.address())?);
    let (agent, updates) = Agent::start(config);
    Ok((agent, listener, updates))
}

/// Carry out what a keypress asked the agent for.
///
/// Not `async`, and that is the point: handing a prompt over is now a
/// non-blocking `try_send`, so this can never park the event loop waiting on
/// an agent that has not finished shaking hands.
fn dispatch(app: &mut App, agent: Option<&Agent>, request: AgentRequest) {
    let Some(agent) = agent else {
        // The editor already said so in the transcript; there is nothing to
        // send and nothing further to report.
        return;
    };

    let outcome = match request {
        AgentRequest::Prompt(text) => agent.prompt(text),
        AgentRequest::Cancel => agent.cancel(),
    };

    match outcome {
        // Handed over. Only now is it working — saying so at the keypress
        // would have claimed a turn had started before anything was sent.
        Ok(()) => app.set_agent(AgentStatus::Busy),
        // Still on the last one. The workspace is fine; the request is not.
        Err(error @ Error::AgentBusy) => {
            app.notice = Some(app::Notice::warning(error.to_string()));
        }
        Err(error) => {
            app.transcript.push_notice(error.to_string());
            app.set_agent(AgentStatus::Absent {
                reason: error.to_string(),
            });
        }
    }
}

/// Apply one terminal event.
fn apply(app: &mut App, event: Event) {
    match event {
        // Windows reports press *and* release; acting on both would move
        // every selection two steps at a time.
        Event::Key(key) if key.kind == KeyEventKind::Press => handle(app, key),
        Event::Mouse(mouse) => handle_mouse(app, mouse),
        _ => {}
    }
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
    // Cleared here rather than by the caller, so the flag cannot survive a
    // failed edit and open the editor again on the next frame.
    if !app.take_system_editor_request() {
        return Ok(());
    }

    match edit_externally(&app.draft()) {
        Ok(Some(edited)) => app.set_draft(&edited),
        // The editor exited non-zero, which is how every editor worth the name
        // says the edit was abandoned. The buffer is left exactly as it was.
        Ok(None) => app.notice = Some(app::Notice::info("editor exited without saving")),
        Err(error) => app.notice = Some(app::Notice::warning(format!("editor failed: {error}"))),
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

    // While the agent is working, `ctrl-c` stops the turn rather than the
    // workspace. Pressing it again quits, because by then it is not working.
    if key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && app.transcript.is_busy()
    {
        app.cancel_turn();
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
        // `a` for ask, on the prompt pane, alongside `e` for edit. The same
        // editor, a different buffer and a different destination: `enter`
        // writes the project's prompt, `a` writes a message to the agent.
        // The prompt *is* the instruction. `a` sends it; there is no second
        // buffer to compose in and nothing to type first.
        KeyCode::Char('a') if app.focus == Focus::Prompt => app.send_prompt(),
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
        // Lowercase re-renders what is in memory; uppercase re-reads the file.
        // Not `ctrl-r`, which is redo inside the prompt editor and worth more
        // there than a second way to reach this.
        KeyCode::Char('R') => app.reload_from_disk(),
        // The picture, the SVG that produced it, or what the agent has been
        // doing. Pressing it also stops the workspace choosing on your behalf.
        KeyCode::Char('s') => app.cycle_view(),
        KeyCode::PageDown if app.scrolls() => app.scroll_view(10),
        KeyCode::PageUp if app.scrolls() => app.scroll_view(-10),
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
    fn a_sends_the_prompt_rather_than_opening_an_editor() {
        // The prompt *is* the instruction. There is no second buffer to
        // compose in — pressing `a` asks the agent to make the artwork match
        // what the pane already says.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Char('a'));

        assert!(!app.is_editing(), "`a` opened an editor instead of sending");
        assert!(matches!(
            app.pending_agent_request,
            Some(AgentRequest::Prompt(_))
        ));
    }

    #[test]
    fn sending_leaves_the_prompt_where_it_was() {
        // It is the project's description of itself, not a message that has
        // been posted. Clearing it would delete the project's metadata every
        // time somebody asked for a render.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;
        let before = app.project.metadata().prompt.clone();

        press(&mut app, KeyCode::Char('a'));

        assert_eq!(app.project.metadata().prompt, before);
        assert_eq!(app.draft(), before.unwrap_or_default());
    }

    #[test]
    fn what_is_sent_tells_the_agent_what_to_do_with_the_prompt() {
        // A bare description gives a model nothing to do, and the likeliest
        // reply is agreement rather than an edit.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Char('a'));

        let Some(AgentRequest::Prompt(sent)) = &app.pending_agent_request else {
            panic!("nothing was sent");
        };
        assert!(sent.contains("write_svg"), "{sent}");
        assert!(sent.contains("render_svg"), "{sent}");
        assert!(
            sent.contains(&app.project.metadata().prompt.clone().unwrap()),
            "the prompt itself was not included: {sent}"
        );
    }

    #[test]
    fn alt_a_sends_without_leaving_the_editor() {
        // `a` is a letter once the editor has the keyboard, so the chord is
        // the way to send something you have just finished typing.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;
        press(&mut app, KeyCode::Enter);

        handle(
            &mut app,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT),
        );

        assert!(app.is_editing(), "it should stay in the editor");
        assert!(matches!(
            app.pending_agent_request,
            Some(AgentRequest::Prompt(_))
        ));
    }

    #[test]
    fn a_plain_a_is_still_a_letter_inside_the_editor() {
        let mut app = app();
        app.focus = Focus::Prompt;
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('a'));

        assert!(app.draft().contains('a'));
        assert_eq!(app.pending_agent_request, None);
    }

    #[test]
    fn enter_in_the_prompt_is_a_newline_rather_than_a_send() {
        // A prompt is a paragraph and needs its line breaks.
        let mut app = app();
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Enter);
        let before = app.draft();
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.draft().lines().count(), before.lines().count() + 1);
        assert_eq!(app.pending_agent_request, None);
    }

    #[test]
    fn a_on_another_pane_does_nothing() {
        for focus in [Focus::Palette, Focus::Variants, Focus::Renders] {
            let mut app = app();
            app.agent = AgentStatus::Ready;
            app.focus = focus;
            press(&mut app, KeyCode::Char('a'));
            assert_eq!(app.pending_agent_request, None, "{focus:?}");
        }
    }

    #[test]
    fn an_empty_prompt_is_refused_with_a_reason() {
        // Silently doing nothing is how the last confusion started.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;
        // Through the editor, because sending commits the buffer first — and
        // a buffer that still held the old prompt would put it straight back.
        app.set_draft("");

        press(&mut app, KeyCode::Char('a'));

        assert_eq!(app.pending_agent_request, None);
        let said = app
            .notice
            .as_ref()
            .and_then(app::Notice::text)
            .unwrap_or("");
        assert!(said.contains("no prompt"), "{said}");
    }

    #[test]
    fn sending_without_an_agent_says_so_rather_than_looking_ignored() {
        let mut app = app();
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Char('a'));

        let said = format!("{:?}", app.transcript.entries());
        assert!(said.contains("no agent"), "{said}");
        assert_eq!(app.pending_agent_request, None);
    }

    /// Feed the workspace an update the way the event loop does.
    ///
    /// Not a helper for its own sake: the *order* of these two steps is the
    /// bug this guards, so a test that did them in the other order would
    /// prove nothing.
    fn deliver(app: &mut App, update: crate::acp::AgentUpdate) {
        let said = match &update {
            crate::acp::AgentUpdate::Ready => Some(AgentStatus::Ready),
            crate::acp::AgentUpdate::Failed(reason) => Some(AgentStatus::Absent {
                reason: reason.clone(),
            }),
            _ => None,
        };
        app.transcript.apply(update);
        app.set_agent(said.unwrap_or(if app.transcript.is_busy() {
            AgentStatus::Busy
        } else {
            AgentStatus::Ready
        }));
    }

    #[test]
    fn the_agent_stops_looking_busy_when_the_turn_ends() {
        // The status used to be derived before the update was applied, so
        // `Idle` — the update that ends a turn — read a transcript that was
        // still busy and latched `Busy`. Nothing ever cleared it: the spinner
        // span and the pane title said "working…" for the rest of the session.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Char('a'));
        app.pending_agent_request = None;
        deliver(&mut app, crate::acp::AgentUpdate::Idle);

        assert_eq!(app.agent, AgentStatus::Ready);
        assert!(!app.is_asking());
        assert_eq!(app.view(), app::View::Preview);
    }

    #[test]
    fn s_cycles_through_the_three_views() {
        let mut app = app();
        assert_eq!(app.view(), app::View::Preview);

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.view(), app::View::Source);

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.view(), app::View::Log);

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.view(), app::View::Preview);
    }

    #[test]
    fn the_log_appears_while_the_agent_works_and_the_preview_when_it_finishes() {
        // Nobody should have to know to press a key to watch a turn happen.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;
        assert_eq!(app.view(), app::View::Preview);

        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.view(), app::View::Log, "the turn is invisible");

        // The turn ends, and the thing worth looking at is the artwork.
        app.pending_agent_request = None;
        app.transcript.apply(crate::acp::AgentUpdate::Idle);
        assert_eq!(app.view(), app::View::Preview);
    }

    #[test]
    fn choosing_a_view_stops_the_workspace_choosing_for_you() {
        // Automatic behaviour that overrides a deliberate choice is worse than
        // none at all.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.view(), app::View::Source);

        press(&mut app, KeyCode::Char('a'));
        assert_eq!(
            app.view(),
            app::View::Source,
            "the workspace overrode a view the user had picked"
        );
    }

    #[test]
    fn the_first_press_moves_on_from_whatever_is_on_screen() {
        // Otherwise pressing `s` while the log is up automatically would jump
        // somewhere unrelated.
        let mut app = app();
        app.agent = AgentStatus::Ready;
        app.focus = Focus::Prompt;
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.view(), app::View::Log);

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.view(), app::View::Preview);
    }

    #[test]
    fn the_source_shown_is_the_one_the_agent_would_read() {
        // `to_svg`, not `source`: an unsaved prompt edit is in the document
        // `get_svg` hands the agent, so it has to be in the one on screen.
        // Otherwise the view cannot answer the question it exists for.
        let mut app = app();
        app.project.metadata_mut().prompt = Some("a wordless circular mark".to_owned());

        assert!(app.source_text().contains("a wordless circular mark"));
    }

    #[test]
    fn a_key_release_is_ignored_so_a_selection_moves_one_step_not_two() {
        // Windows reports press *and* release. `apply` is where that is
        // filtered now that the loop no longer reads events itself.
        let mut app = app();
        app.focus = Focus::Variants;

        for kind in [KeyEventKind::Press, KeyEventKind::Release] {
            apply(
                &mut app,
                Event::Key(KeyEvent {
                    code: KeyCode::Down,
                    modifiers: KeyModifiers::NONE,
                    kind,
                    state: ratatui::crossterm::event::KeyEventState::NONE,
                }),
            );
        }

        assert_eq!(app.selected_variant(), 1);
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
        let notice = app
            .notice
            .as_ref()
            .and_then(app::Notice::text)
            .unwrap_or_default()
            .to_owned();

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
