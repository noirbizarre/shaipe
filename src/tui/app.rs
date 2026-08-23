//! The workspace's state.
//!
//! Deliberately plain: fields and small methods, no framework. The preview is
//! the only derived state, and it is cached because rendering on every frame
//! would rasterise a 512-pixel SVG four times a second for no reason.
//!
//! This is where editing lives. The prompt editor holds its draft and the
//! dirty flag here rather than spread through the drawing code, and a colour
//! picker or an agent conversation would land beside them.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::ListState;
use ratatui_textarea::{TextArea, WrapMode};

use crate::preview::{Image, Scale};
use crate::project::{Format, Project, RenderSpec, Rgba};
use crate::render::{RenderOptions, Renderer};
use crate::tui::modal::{Modal, RendersEditor};
use crate::tui::render_worker::{Rendered, Worker};
use crate::tui::toolbar::Button;
use crate::tui::transcript::Transcript;
use crate::tui::watch::Watcher;

/// The narrowest the description column may be dragged.
///
/// Below this the hex colours and render sizes no longer fit, and the column
/// stops being readable rather than merely cramped.
pub const MIN_COLUMN: u16 = 24;

/// How much room the preview column always keeps.
pub const MIN_PREVIEW: u16 = 12;

/// How close together two clicks count as one double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The largest a preview is rasterised, before it is fitted to the pane.
const PREVIEW_SIZE: u32 = 512;

/// The smallest a preview is allowed to shrink to.
///
/// Below this a logo stops being recognisable, and the rasterising is cheap
/// enough that shrinking further buys nothing.
const MIN_PREVIEW_SIZE: u32 = 32;

/// How long the selection must sit still before a render starts.
///
/// Long enough that arrowing through a list renders once at the end rather
/// than once per entry; short enough that a single deliberate step still feels
/// immediate.
const DEBOUNCE: Duration = Duration::from_millis(120);

/// Where a double-clicked render specification is written.
///
/// The conventional output directory, matching `shaipe render`'s default, so
/// the workspace and the command line do not disagree about where assets go.
const EXPORT_DIRECTORY: &str = "dist";

/// Which pane the keyboard is talking to.
///
/// Two, not four: the variants and the render specifications are the preview's
/// tabs now, chosen by [`Mode`], and neither is a list to walk on the left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The prompt, or the transcript in its place.
    Prompt,
    /// The palette.
    Palette,
}

impl Focus {
    /// Every pane, in the order `Tab` visits them.
    pub const ALL: [Self; 2] = [Self::Prompt, Self::Palette];

    /// How many panes there are.
    pub const COUNT: usize = Self::ALL.len();

    /// The pane's title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Palette => "palette",
        }
    }
}

/// What the top-left box is showing.
///
/// One box with two contents rather than two panes: the prompt and the
/// transcript are never wanted at the same moment, and a pane each would have
/// left both of them too short to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftView {
    /// The project's own description of itself.
    #[default]
    Prompt,
    /// What the agent has said and done.
    Transcript,
}

impl LeftView {
    /// The other one.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Prompt => Self::Transcript,
            Self::Transcript => Self::Prompt,
        }
    }

    /// What to call it, in a pane title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Transcript => "transcript",
        }
    }
}

/// What the preview's tabs are listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// One tab per variant, previewed at a default size.
    #[default]
    Variants,
    /// One tab per render specification, previewed exactly as declared.
    Renders,
}

impl Mode {
    /// The other one.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Variants => Self::Renders,
            Self::Renders => Self::Variants,
        }
    }

    /// What to call it, on the toolbar.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Variants => "variants",
            Self::Renders => "renders",
        }
    }

    /// Its position in `tab`.
    const fn index(self) -> usize {
        match self {
            Self::Variants => 0,
            Self::Renders => 1,
        }
    }
}

/// Which part of a colour the palette editor is changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteField {
    /// What the project calls it.
    Name,
    /// The colour itself.
    #[default]
    Value,
}

impl PaletteField {
    /// The other one.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Name => Self::Value,
            Self::Value => Self::Name,
        }
    }
}

/// A committed palette edit, once the buffer has been understood.
///
/// The parsing happens before the project is borrowed mutably, so a field that
/// says nothing usable costs a `return` rather than a half-applied change.
enum Edit {
    /// A new name for the colour.
    Name(String),
    /// A new value for it.
    Value(Rgba),
}

/// A field of the palette being edited, and whether it currently parses.///
/// Handed to the pane so the half-typed text is what is drawn. Without it the
/// pane would show the committed value and the keystrokes would be invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEdit<'a> {
    /// Which colour.
    pub row: usize,
    /// Which of its fields.
    pub field: PaletteField,
    /// What has been typed so far.
    pub text: &'a str,
    /// Whether that text is something the project could hold.
    pub valid: bool,
}

/// What the preview pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    /// Not rendered yet.
    Pending,
    /// A rendered image, and what it was rendered from.
    Ready {
        /// The pixels.
        image: Box<Image>,
        /// A description of what produced them.
        caption: String,
    },
    /// Rendering failed. Shown in the pane rather than taking the workspace
    /// down: a project being edited towards correctness spends time broken,
    /// and quitting on every intermediate state would make it unusable.
    Failed(String),
}

/// Whether the workspace has an agent, and what it is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    /// There is none. The workspace still works, and the prompt pane says why
    /// it cannot be used. An agent that will not start must not stop someone
    /// from reading their own project.
    Absent {
        /// What to tell them.
        reason: String,
    },
    /// Started, and still shaking hands.
    ///
    /// The workspace does not wait for this to finish — the agent asks
    /// Shaipe's own MCP server for a tool list before it answers `session/new`,
    /// and only the event loop can answer that. So the workspace opens, this
    /// is what it opens with, and a prompt typed now waits for `Ready`.
    Connecting,
    /// Started, and waiting for a prompt.
    Ready,
    /// Working on a turn.
    Busy,
}

impl AgentStatus {
    /// Whether a prompt can be sent right now.
    #[must_use]
    pub const fn accepts_a_prompt(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// How long a notice stays on screen.
///
/// Long enough to read, short enough that the keys come back on their own.
/// Nothing cleared a notice before, and since the status line shows one
/// *instead of* the hints, a single save removed every key hint for the rest
/// of the session — including the one that reaches the agent.
const NOTICE: Duration = Duration::from_secs(4);

/// How long a failure stays on screen.
///
/// Longer, because "wrote logo.svg" is a confirmation and "save failed: …" is
/// something to act on.
const WARNING: Duration = Duration::from_secs(12);

/// Something the workspace has to say, and when it said it.
#[derive(Debug, Clone)]
pub struct Notice {
    text: String,
    raised: Instant,
    lifetime: Duration,
}

impl Notice {
    /// Something that went as intended.
    #[must_use]
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            raised: Instant::now(),
            lifetime: NOTICE,
        }
    }

    /// Something that did not, and stays longer for it.
    #[must_use]
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            raised: Instant::now(),
            lifetime: WARNING,
        }
    }

    /// What it says, while it still has anything to say.
    ///
    /// `None` once it has aged out, so the caller shows the key hints again
    /// without anything having to clear it. The workspace redraws on a 250 ms
    /// tick, so it goes of its own accord within that.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        (self.raised.elapsed() < self.lifetime).then_some(&self.text)
    }

    /// Whether it is reporting a failure.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.lifetime == WARNING
    }

    /// How long it stays on screen.
    #[must_use]
    pub const fn lifetime(&self) -> Duration {
        self.lifetime
    }

    /// One that has already aged out, for testing what the line does then.
    ///
    /// A constructor rather than sleeping: a test that waits four seconds to
    /// assert a four-second timeout is a test nobody runs.
    #[cfg(test)]
    #[must_use]
    pub fn aged(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            raised: Instant::now() - NOTICE - Duration::from_secs(1),
            lifetime: NOTICE,
        }
    }
}

/// What Shaipe asks the agent when the prompt is sent.
///
/// The prompt is a description of the artwork, not an instruction — sent bare,
/// it gives a model nothing to do, and the likeliest reply is agreement. This
/// says what to do with it, and names the tools, because the preamble is read
/// once and this is read every turn.
fn instruct(prompt: &str) -> String {
    format!(
        "Update this project's SVG so that it matches the prompt below.\n\n\
         Read the current document with `get_svg`, write the new one with \
         `write_svg`, and look at the result with `render_svg` before you \
         finish. Change the artwork only — leave the `<metadata>` block as you \
         found it.\n\n---\n\n{prompt}"
    )
}

/// What the right-hand column is showing.
///
/// Two, not three: the transcript used to be a third view here, and it is the
/// top-left box now — see [`LeftView`]. What is left is the artwork and the
/// document that produced it, which is a toggle rather than a cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// The rendered artwork.
    #[default]
    Preview,
    /// The SVG that produced it.
    Source,
}

impl View {
    /// What to call it, in a pane title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Source => "source",
        }
    }

    /// The other one.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Preview => Self::Source,
            Self::Source => Self::Preview,
        }
    }
}

/// Something a keypress asked the agent to do.
///
/// Recorded by the key handler and carried out by the event loop, so that key
/// handling stays synchronous: making it `async` would mean every test that
/// asserts `Esc` moves the focus needed a runtime to do it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRequest {
    /// Send a turn.
    Prompt(String),
    /// Stop the turn in progress.
    Cancel,
}

/// The workspace.
pub struct App {
    /// The project being worked on.
    pub project: Project,
    /// Which pane has the keyboard.
    pub focus: Focus,
    /// Whether the event loop should stop.
    pub should_quit: bool,
    /// The name of the preview backend in use, shown in the status line so a
    /// preview that looks wrong is diagnosable without a debugger.
    pub backend: &'static str,
    /// How much detail to show. Only the render timing depends on it.
    pub verbose: u8,

    /// How wide the description column is. Draggable.
    pub column_width: u16,
    /// A transient message, shown in the status line until it ages out.
    pub notice: Option<Notice>,

    /// The prompt editor's buffer and cursor.
    ///
    /// Seeded from the project and committed back on every keystroke, so the
    /// project is always what the pane shows. It survives disengaging, so
    /// leaving the editor and coming back keeps the cursor where it was.
    editor: TextArea<'static>,
    /// Which pane the keyboard is editing, if any.
    ///
    /// One edit mode shared by the prompt and the palette, rather than a flag
    /// each. It is explicit rather than implied by the focus, because every
    /// printable key is an edit while a pane is being edited, `q` and `r`
    /// among them: there has to be a state in which a pane is focused and the
    /// workspace's own shortcuts still work.
    editing: Option<Focus>,
    /// Which part of the selected colour the palette editor is changing.
    palette_field: PaletteField,
    /// What has been typed into that field so far.
    ///
    /// A buffer rather than editing the project in place, because a colour is
    /// unparsable for most of the time it takes to type one: `#f0` is not a
    /// value the project can hold, and refusing the keystroke would make the
    /// field impossible to use.
    palette_buffer: String,
    /// Whether `$EDITOR` has been asked for and not yet opened.
    ///
    /// A flag rather than the deed, because opening an editor needs the
    /// terminal, which the state deliberately does not have.
    editor_requested: bool,
    /// Whether the project has changes that are not on disk.
    dirty: bool,
    /// Whether a quit was refused because of [`App::dirty`].
    confirm_quit: bool,
    /// Whether the file has changed since the workspace last read or wrote it.
    watcher: Watcher,
    /// Whether somebody else's write is waiting to be taken.
    ///
    /// Set when the file changes while there is unsaved work, which is the one
    /// case the workspace must not resolve on its own: both versions are
    /// somebody's, and picking is a decision.
    stale: bool,
    /// What the agent has said and done.
    pub transcript: Transcript,
    /// Whether there is an agent at all, and if not, why not.
    pub agent: AgentStatus,
    /// What the last keypress asked the agent for, if anything.
    pub pending_agent_request: Option<AgentRequest>,
    /// When the agent last changed what it was doing.
    ///
    /// Only the spinner reads it, which derives its frame from elapsed time
    /// rather than storing one.
    agent_since: Instant,
    /// Which view the right-hand column is showing.
    ///
    /// A plain state, toggled by `s`. Nothing chooses it on the user's behalf:
    /// the transcript, which was the one view worth showing automatically, is
    /// in the left column now and has its own rule — see [`App::follow_the_turn`].
    view: View,
    /// Which of the prompt and the transcript the top-left box is showing.
    left: LeftView,
    /// Whether a turn was running last time the box was asked to follow one.
    ///
    /// Edge-triggered rather than derived, so that toggling with `t` during a
    /// turn is not undone on the very next frame.
    was_asking: bool,
    /// What the preview's tabs are listing.
    mode: Mode,
    /// The selected tab, per mode.
    ///
    /// One each, so switching to the render specifications and back returns to
    /// the variant that was on screen rather than to the first one.
    tab: [usize; 2],
    /// How far down the source is scrolled, in rows.
    view_scroll: u16,
    /// How far back through the transcript the reader has gone, in rows.
    transcript_scroll: u16,
    /// The modal covering the workspace, if any.
    modal: Option<Modal>,
    /// Where each toolbar button was drawn last frame.
    toolbar: Vec<(Button, Rect)>,
    /// Where each preview tab was drawn last frame.
    tabs: Vec<Rect>,

    /// Where each pane was drawn last frame.
    ///
    /// Recorded during drawing rather than recomputed when a click arrives:
    /// two copies of the layout arithmetic would drift, and the bug that
    /// caused would be a click landing on the wrong pane.
    areas: [Rect; Focus::COUNT],
    preview_area: Rect,
    /// The preview pane's size in pixels, or `(0, 0)` before the first frame.
    preview_pixels: (u32, u32),
    /// The last click, for recognising a double-click.
    last_click: Option<(Focus, u16, Instant)>,
    /// The last click on a tab, for the same reason.
    last_tab_click: Option<(usize, Instant)>,
    /// Whether the column divider is being dragged.
    resizing: bool,
    /// Whether to skip drawing the image for one frame.
    holding_image: bool,
    /// Scroll state per list pane, so the selection stays visible when a pane
    /// is smaller than its contents.
    list_states: [ListState; Focus::COUNT],

    selected: [usize; Focus::COUNT],
    preview: Preview,
    /// What the on-screen preview was rendered from.
    rendered_from: Option<RenderSpec>,
    /// What the selection currently calls for.
    desired: Option<RenderSpec>,
    /// When `desired` last changed, for the debounce.
    changed_at: Instant,
    /// The request the worker is busy with, and when it was sent.
    in_flight: Option<(u64, RenderSpec, Instant)>,
    /// The last request number handed out.
    sequence: u64,
    /// How long the last completed render took.
    last_render: Option<Duration>,
    /// Renders previews without blocking the drawing thread.
    worker: Worker,
    /// Bumped whenever the preview image is replaced.
    ///
    /// The preview backend keeps the image in encoded form and must not be
    /// asked to compare pixels to notice a change; a counter is both cheaper
    /// and impossible to get subtly wrong.
    generation: u64,
}

impl App {
    /// Open a project in the workspace.
    #[must_use]
    pub fn new(project: Project, backend: &'static str) -> Self {
        let project_for_worker = project.clone();
        let editor = Self::editor_for(project.metadata().prompt.as_deref().unwrap_or_default());
        Self {
            project,
            focus: Focus::Prompt,
            should_quit: false,
            backend,
            verbose: 0,
            column_width: 0,
            notice: None,
            editor,
            editing: None,
            palette_field: PaletteField::default(),
            palette_buffer: String::new(),
            editor_requested: false,
            dirty: false,
            confirm_quit: false,
            watcher: Watcher::new(project_for_worker.path()),
            stale: false,
            transcript: Transcript::default(),
            agent: AgentStatus::Absent {
                reason: "no agent was started".to_owned(),
            },
            pending_agent_request: None,
            agent_since: Instant::now(),
            view: View::default(),
            left: LeftView::default(),
            was_asking: false,
            mode: Mode::default(),
            tab: [0; 2],
            view_scroll: 0,
            transcript_scroll: 0,
            modal: None,
            toolbar: Vec::new(),
            tabs: Vec::new(),
            areas: [Rect::ZERO; Focus::COUNT],
            preview_area: Rect::ZERO,
            preview_pixels: (0, 0),
            last_click: None,
            last_tab_click: None,
            resizing: false,
            holding_image: false,
            list_states: std::array::from_fn(|_| ListState::default()),
            selected: [0; Focus::COUNT],
            preview: Preview::Pending,
            rendered_from: None,
            desired: None,
            // In the past, so the first preview starts at once rather than
            // making the workspace wait out a debounce it has no reason to.
            changed_at: Instant::now() - DEBOUNCE,
            in_flight: None,
            sequence: 0,
            last_render: None,
            worker: Worker::new(&project_for_worker),
            generation: 0,
        }
    }

    /// The index of the focused pane, for indexing `selected`.
    fn focus_index(&self) -> usize {
        Self::index_of(self.focus)
    }

    /// A pane's position in the fixed pane order.
    fn index_of(focus: Focus) -> usize {
        Focus::ALL
            .iter()
            .position(|candidate| *candidate == focus)
            .unwrap_or_default()
    }

    /// How many selectable rows the focused pane has.
    fn focused_len(&self) -> usize {
        self.pane_len(self.focus)
    }

    /// Move the keyboard to the next pane.
    ///
    /// Keeps the edit mode: `tab` says which pane the arrows belong to, and
    /// dropping out of editing on the way would make moving between two
    /// editable panes cost an extra keystroke every time.
    pub fn focus_next(&mut self) {
        self.move_focus(Focus::ALL[(self.focus_index() + 1) % Focus::COUNT]);
    }

    /// Move the keyboard to the previous pane.
    pub fn focus_previous(&mut self) {
        self.move_focus(Focus::ALL[(self.focus_index() + Focus::COUNT - 1) % Focus::COUNT]);
    }

    /// Move the keyboard to a named pane, taking the edit mode with it.
    fn move_focus(&mut self, focus: Focus) {
        let editing = self.editing.is_some();
        // Committed on the way out rather than on arrival: the buffer belongs
        // to the pane being left, and carrying it across would write a colour
        // name into the prompt.
        self.leave_edit();
        self.focus = focus;
        if editing {
            self.engage_editor();
        }
    }

    /// A text area holding `text`, styled the way the prompt pane wants it.
    ///
    /// Word wrapping because a prompt is prose, falling back to breaking a
    /// word that is wider than the column rather than letting it disappear off
    /// the edge. The cursor line's underline is cleared: the crate draws it to
    /// mark the current line of source, and in a paragraph it reads as an
    /// artefact.
    fn editor_for(text: &str) -> TextArea<'static> {
        let mut editor = TextArea::from(text.lines());
        editor.set_wrap_mode(WrapMode::WordOrGlyph);
        editor.set_cursor_line_style(Style::default());
        editor
    }

    /// Hand the keyboard to the focused pane's editor.
    ///
    /// One mode for two panes. The prompt's editor is a text area; the
    /// palette's is a field on the selected colour. What they share is that
    /// while either is engaged the arrows belong to the pane and every
    /// printable key is an edit.
    pub fn engage_editor(&mut self) {
        self.editing = Some(self.focus);
        if self.focus == Focus::Palette {
            self.load_palette_field();
        }
    }

    /// Take the keyboard back.
    pub fn disengage_editor(&mut self) {
        self.leave_edit();
    }

    /// Leave whichever editor is engaged, committing what it holds.
    fn leave_edit(&mut self) {
        match self.editing.take() {
            Some(Focus::Prompt) => self.commit_prompt(),
            Some(Focus::Palette) => self.commit_palette_field(),
            None => {}
        }
    }

    /// Whether keys are going to a pane's editor.
    #[must_use]
    pub const fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// Which pane is being edited, if any.
    #[must_use]
    pub const fn editing(&self) -> Option<Focus> {
        self.editing
    }

    /// Whether the prompt's text area has the keyboard.
    ///
    /// Distinct from [`Self::is_editing`], which the palette also satisfies:
    /// only this one means the text area is what a keypress should reach.
    #[must_use]
    pub fn is_editing_prompt(&self) -> bool {
        self.editing == Some(Focus::Prompt)
    }

    /// Whether the project holds changes that are not on disk.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Note that the project has changed and is no longer what is on disk.
    ///
    /// For changes that did not come from the editor — an agent writing an SVG
    /// through a tool, most of all. Separate from
    /// [`Self::invalidate_preview`], which is about pixels: a change can want
    /// a re-render, a save, or both, and conflating them would mean pressing
    /// `r` marked the project dirty.
    pub const fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Note that the project this app holds has never been saved to disk.
    ///
    /// Called once, right after construction, by `shaipe::tui::run` when it
    /// was handed a path that did not exist — see `Project::init`. Kept
    /// separate from [`Self::new`] rather than folded into it, so every
    /// existing call site building an app for a project that really is on
    /// disk stays exactly as it was: unaffected, and not reaching the
    /// filesystem a second time to ask a question its caller already
    /// answered.
    pub fn mark_as_new_project(&mut self) {
        self.dirty = true;
        self.notice = Some(Notice::info(format!(
            "new project — ctrl-s to save {}",
            self.project.path().display()
        )));
    }

    /// Re-read the prompt from the project into the editor.
    ///
    /// After something other than the editor changed it — an agent calling
    /// `write_svg` with a new `<shaipe:prompt>`. The buffer is a copy, so
    /// without this the next keystroke would write the stale one back.
    ///
    pub fn reload_prompt(&mut self) {
        let prompt = self.project.metadata().prompt.clone().unwrap_or_default();
        if prompt != self.draft() {
            self.editor = Self::editor_for(&prompt);
        }
    }

    /// Whether a quit is waiting on confirmation.
    #[must_use]
    pub const fn wants_quit_confirmation(&self) -> bool {
        self.confirm_quit
    }

    /// The prompt editor, for drawing.
    #[must_use]
    pub const fn editor(&self) -> &TextArea<'static> {
        &self.editor
    }

    /// What the prompt editor currently holds.
    #[must_use]
    pub fn draft(&self) -> String {
        self.editor.lines().join("\n")
    }

    /// Replace what the prompt editor holds, as `$EDITOR` returning does.
    ///
    /// A fresh text area rather than an edited one, so the styling stays in
    /// one place and the cursor cannot be left pointing past the new end.
    pub fn set_draft(&mut self, text: &str) {
        self.editor = Self::editor_for(text);
        self.commit_prompt();
    }

    /// Apply a keypress to whichever pane is being edited.
    ///
    /// Three keys are taken before the pane sees them, and they are the same
    /// three for both panes, which is the point of having one edit mode. `esc`
    /// leaves; `tab` and `shift-tab` move to the next pane and keep editing —
    /// tab is how the workspace is navigated, and a literal tab in a paragraph
    /// of prose is worth much less than a consistent way out. Everything else
    /// belongs to the pane, arrows included.
    pub fn edit_key(&mut self, key: KeyEvent) {
        // Taken before the text area sees it. `alt+a` is free in its key map,
        // where `alt+f`, `alt+b`, `alt+d` and `alt+h` are not, and unlike
        // `alt+enter` it cannot be swallowed: the text area matches
        // `Key::Enter, ..`, which ignores every modifier.
        //
        // The same thing `a` does from the pane, without having to leave the
        // editor to do it.
        if key.code == KeyCode::Char('a') && key.modifiers.contains(KeyModifiers::ALT) {
            self.send_prompt();
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.leave_edit();
                return;
            }
            KeyCode::Tab => {
                self.focus_next();
                return;
            }
            KeyCode::BackTab => {
                self.focus_previous();
                return;
            }
            _ => {}
        }

        match self.editing {
            Some(Focus::Palette) => self.palette_key(key),
            _ => {
                self.editor.input(key);
                self.commit_prompt();
            }
        }
    }

    /// Apply a keypress to the palette's field editor.
    ///
    /// The arrows belong to the pane: up and down change which colour is being
    /// edited, left and right which of its fields. Each of those commits what
    /// is in the buffer first, so moving away from a field is as good as
    /// finishing it.
    fn palette_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.commit_palette_field();
                self.select_previous();
                self.load_palette_field();
            }
            KeyCode::Down => {
                self.commit_palette_field();
                self.select_next();
                self.load_palette_field();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Enter => {
                self.commit_palette_field();
                self.palette_field = self.palette_field.other();
                self.load_palette_field();
            }
            KeyCode::Backspace => {
                self.palette_buffer.pop();
                self.commit_palette_field();
            }
            KeyCode::Char(character) => {
                self.palette_buffer.push(character);
                self.commit_palette_field();
            }
            _ => {}
        }
    }

    /// Fill the buffer from the colour and field now selected.
    fn load_palette_field(&mut self) {
        let row = self.selection(Focus::Palette);
        self.palette_buffer = self
            .project
            .metadata()
            .palette
            .colours()
            .get(row)
            .map(|colour| match self.palette_field {
                PaletteField::Name => colour.name.clone(),
                PaletteField::Value => colour.value.to_string(),
            })
            .unwrap_or_default();
    }

    /// Write the buffer back to the project, if it says something usable.
    ///
    /// An unusable buffer is kept and shown rather than refused: `#f0` is what
    /// every hex colour looks like halfway through being typed, and rejecting
    /// the keystroke would make the field impossible to use. The pane draws it
    /// in red until it parses, and nothing reaches the document until it does.
    fn commit_palette_field(&mut self) {
        let row = self.selection(Focus::Palette);
        let field = self.palette_field;
        let text = self.palette_buffer.trim().to_owned();

        // Parsed before anything is borrowed mutably, and a failure simply
        // leaves the document alone.
        let value = match field {
            // An empty name would leave a colour nothing can refer to.
            PaletteField::Name => (!text.is_empty()).then_some(Edit::Name(text)),
            PaletteField::Value => text.parse::<Rgba>().ok().map(Edit::Value),
        };
        let Some(edit) = value else { return };

        // Read before anything is mutated: a value change restyles whatever
        // the artwork already binds under the name the colour has *now*, not
        // whatever it is about to be renamed to.
        let Some(name) = self
            .project
            .metadata()
            .palette
            .colours()
            .get(row)
            .map(|colour| colour.name.clone())
        else {
            return;
        };

        let changed = {
            let palette = &mut self.project.metadata_mut().palette;
            let Some(colour) = palette.colour_mut(row) else {
                return;
            };

            match &edit {
                Edit::Name(new_name) if colour.name != *new_name => {
                    colour.name = new_name.clone();
                    true
                }
                Edit::Value(rgba) if colour.value != *rgba => {
                    colour.value = *rgba;
                    true
                }
                _ => false,
            }
        };

        if changed {
            if let Edit::Value(rgba) = edit {
                self.project.restyle(&name, rgba).expect(
                    "the project's own document already parsed; only bound \
                     attributes change",
                );
            }
            self.dirty = true;
        }
    }

    /// The palette field being edited, for the pane to draw.
    #[must_use]
    pub fn palette_edit(&self) -> Option<PaletteEdit<'_>> {
        (self.editing == Some(Focus::Palette)).then(|| PaletteEdit {
            row: self.selection(Focus::Palette),
            field: self.palette_field,
            text: &self.palette_buffer,
            valid: match self.palette_field {
                PaletteField::Name => !self.palette_buffer.trim().is_empty(),
                PaletteField::Value => self.palette_buffer.trim().parse::<Rgba>().is_ok(),
            },
        })
    }

    /// Ask the agent to make the artwork match the project's prompt.
    ///
    /// The prompt *is* the instruction — there is no second buffer and no
    /// conversation. What is on screen is what is sent, and it stays on screen
    /// afterwards, because it is the project's own description of itself and
    /// not a message that has been posted.
    ///
    /// Records the request rather than sending it: sending needs an `await`,
    /// and making key handling asynchronous would mean every test asserting
    /// that `esc` moves the focus needed a runtime to do it.
    pub fn send_prompt(&mut self) {
        // Committed first, so what is sent is what is on screen rather than
        // what was on screen when the editor was last left.
        self.commit_prompt();

        let Some(prompt) = self.project.metadata().prompt.clone() else {
            self.notice = Some(Notice::warning(
                "there is no prompt to send — press enter and describe the artwork",
            ));
            return;
        };

        if let AgentStatus::Absent { reason } = &self.agent {
            let reason = reason.clone();
            self.transcript
                .push_notice(format!("no agent is connected: {reason}"));
            return;
        }

        self.transcript.push_user(prompt.clone());
        self.pending_agent_request = Some(AgentRequest::Prompt(instruct(&prompt)));
    }

    /// What the right-hand column is showing.
    #[must_use]
    pub const fn view(&self) -> View {
        self.view
    }

    /// Swap the artwork for the document that produced it, or back.
    pub const fn toggle_view(&mut self) {
        self.view = self.view.other();
        self.view_scroll = 0;
    }

    /// Which of the prompt and the transcript the top-left box is showing.
    #[must_use]
    pub const fn left_view(&self) -> LeftView {
        self.left
    }

    /// Swap one for the other.
    pub const fn toggle_left_view(&mut self) {
        self.left = self.left.other();
        self.transcript_scroll = 0;
    }

    /// Show whichever of the two the turn makes interesting.
    ///
    /// The transcript when one starts, the prompt when it ends. Edge-triggered
    /// on purpose: the box is a plain state that `t` toggles at any moment, and
    /// a rule derived from `is_asking` on every frame would undo that toggle
    /// before it reached the screen.
    pub fn follow_the_turn(&mut self) {
        let asking = self.is_asking();
        if asking != self.was_asking {
            self.was_asking = asking;
            self.left = if asking {
                LeftView::Transcript
            } else {
                LeftView::Prompt
            };
            self.transcript_scroll = 0;
        }
    }

    /// What the preview's tabs are listing.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Swap the variants for the render specifications, or back.
    pub const fn toggle_mode(&mut self) {
        self.mode = self.mode.other();
    }

    /// The names on the tab bar, in order.
    #[must_use]
    pub fn tabs(&self) -> Vec<String> {
        let metadata = self.project.metadata();
        match self.mode {
            Mode::Variants => metadata
                .variants
                .iter()
                .map(|variant| variant.name.clone())
                .collect(),
            Mode::Renders => metadata
                .renders
                .iter()
                .map(|spec| spec.name.clone())
                .collect(),
        }
    }

    /// Which tab is selected.
    ///
    /// Clamped rather than trusted: an agent's edit can delete the variant
    /// that was on screen, and an out-of-range index would panic on draw.
    #[must_use]
    pub fn tab(&self) -> usize {
        self.tab[self.mode.index()].min(self.tab_count().saturating_sub(1))
    }

    /// How many tabs the current mode has.
    fn tab_count(&self) -> usize {
        let metadata = self.project.metadata();
        match self.mode {
            Mode::Variants => metadata.variants.len(),
            Mode::Renders => metadata.renders.len(),
        }
    }

    /// Show the next tab, wrapping past the last.
    pub fn next_tab(&mut self) {
        self.step_tab(1);
    }

    /// Show the previous tab, wrapping past the first.
    pub fn previous_tab(&mut self) {
        self.step_tab(-1);
    }

    /// Show a tab by position, ignoring one that is not there.
    pub fn select_tab(&mut self, index: usize) {
        if index < self.tab_count() {
            self.tab[self.mode.index()] = index;
        }
    }

    /// Move the tab selection, wrapping at both ends.
    ///
    /// Wrapping because a project has a handful of variants, and reaching the
    /// second from the last by going forwards is quicker than noticing that
    /// the end has been reached.
    fn step_tab(&mut self, delta: isize) {
        let count = self.tab_count();
        if count == 0 {
            return;
        }
        let current = self.tab() as isize;
        self.tab[self.mode.index()] = (current + delta).rem_euclid(count as isize) as usize;
    }

    /// Whether something on screen scrolls.
    #[must_use]
    pub fn scrolls(&self) -> bool {
        self.view == View::Source || self.left == LeftView::Transcript
    }

    /// How far down the source is scrolled.
    #[must_use]
    pub const fn view_scroll(&self) -> u16 {
        self.view_scroll
    }

    /// How far back through the transcript the reader has gone.
    #[must_use]
    pub const fn transcript_scroll(&self) -> u16 {
        self.transcript_scroll
    }

    /// Scroll by a page, in whichever direction.
    ///
    /// Whichever of the two scrollable things is on screen; the source wins
    /// when both are, because the right column is the larger of the two and
    /// the one the eye is on.
    ///
    /// Clamped at the top; the bottom is left to the widget, which simply
    /// draws nothing past the end.
    pub const fn scroll_view(&mut self, rows: i16) {
        if matches!(self.view, View::Source) {
            self.view_scroll = self.view_scroll.saturating_add_signed(rows);
        } else {
            // The transcript is anchored to its newest entry, so scrolling
            // "down" is going back through it.
            self.transcript_scroll = self.transcript_scroll.saturating_add_signed(rows);
        }
    }

    /// The document as it now stands, for the source view.
    ///
    /// `to_svg` rather than `source`, so what is read here is exactly what
    /// `get_svg` hands the agent — including a prompt edit that has not been
    /// saved. Seeing whether anything actually changed is the whole point of
    /// the view, and a stale copy would defeat it.
    #[must_use]
    pub fn source_text(&self) -> String {
        self.project
            .to_svg()
            .unwrap_or_else(|error| format!("could not render the document: {error}"))
    }

    /// When the agent last changed what it was doing.
    #[must_use]
    pub const fn agent_since(&self) -> Instant {
        self.agent_since
    }

    /// Note what the agent is doing now.
    ///
    /// Restarts the spinner, so a new turn does not inherit the phase of the
    /// last one.
    pub fn set_agent(&mut self, status: AgentStatus) {
        if self.agent != status {
            self.agent_since = Instant::now();
            self.agent = status;
        }
    }

    /// Whether a prompt is on its way to the agent, or being worked on.
    #[must_use]
    pub const fn is_asking(&self) -> bool {
        self.pending_agent_request.is_some() || self.transcript.is_busy()
    }

    /// Ask the agent to stop the turn it is on.
    pub fn cancel_turn(&mut self) {
        self.pending_agent_request = Some(AgentRequest::Cancel);
    }

    /// Apply a mouse event to the prompt editor.
    ///
    /// Only the wheel does anything: the crate has no way to place the cursor
    /// at a screen cell, so a click inside the pane would have nothing to do.
    pub fn edit_mouse(&mut self, mouse: MouseEvent) {
        self.editor.input(mouse);
        self.commit_prompt();
    }

    /// Ask for the prompt to be opened in `$EDITOR`.
    ///
    /// Raises a flag rather than doing it: opening an editor means giving the
    /// terminal away and taking it back, and the state has no terminal.
    pub const fn request_system_editor(&mut self) {
        self.editor_requested = true;
    }

    /// Take a pending `$EDITOR` request, if there is one.
    pub const fn take_system_editor_request(&mut self) -> bool {
        std::mem::replace(&mut self.editor_requested, false)
    }

    /// Whether `$EDITOR` has been asked for and not yet opened.
    #[must_use]
    pub const fn wants_system_editor(&self) -> bool {
        self.editor_requested
    }

    /// Copy the editor's buffer into the project.
    ///
    /// Called after every keystroke, which is a string comparison rather than
    /// a render: the prompt is metadata and has no effect on geometry, so an
    /// edit never invalidates the preview.
    ///
    /// The text is trimmed and an empty prompt becomes `None`, for two
    /// reasons. The reader trims (`Metadata::from_document`), so an untrimmed
    /// write would not survive a round trip; and a prompt cleared to nothing
    /// must drop the element rather than write an empty one, which is the same
    /// omit-the-default rule that lets an untouched project be saved without
    /// changing a byte.
    pub fn commit_prompt(&mut self) {
        let text = self.draft();
        let text = text.trim();
        let prompt = (!text.is_empty()).then(|| text.to_owned());

        if self.project.metadata().prompt != prompt {
            self.project.metadata_mut().prompt = prompt;
            self.dirty = true;
        }
    }

    /// Write the project back to its file.
    ///
    /// The outcome goes to the status line rather than being returned: a
    /// failed write must not close the workspace, which is the one place the
    /// unsaved work still exists.
    pub fn save(&mut self) {
        // Refused rather than resolved. Writing here would overwrite somebody
        // else's work — very likely an agent's, since nothing over ACP can
        // stop one using its own editor — with bytes read before they wrote.
        // `R` takes theirs; saving again takes yours.
        if self.stale {
            self.stale = false;
            self.notice = Some(Notice::warning(format!(
                "{} changed on disk — R to take it, or ctrl-s again to overwrite it",
                self.project.path().display()
            )));
            return;
        }

        self.commit_prompt();
        self.notice = Some(match self.project.save() {
            Ok(()) => Notice::info({
                self.dirty = false;
                self.confirm_quit = false;
                // Its own write must not come back as somebody else's change.
                self.watcher.accept(self.project.path());
                format!("wrote {}", self.project.path().display())
            }),
            Err(error) => Notice::warning(format!("save failed: {error}")),
        });
    }

    /// Notice, and act on, the file changing underneath the workspace.
    ///
    /// Called from the event loop's tick. Reads two fields of one `stat`, and
    /// only when they differ does it do anything more.
    ///
    /// With nothing to lose, the change is taken: that is what makes an agent
    /// reaching for its own editor — which nothing over ACP can prevent —
    /// still update the preview. With unsaved work it is *not* taken, because
    /// both versions are somebody's and choosing between them is a decision,
    /// not a default.
    pub fn poll_file(&mut self) {
        if !self.watcher.changed(self.project.path()) {
            return;
        }

        if self.dirty {
            self.stale = true;
            self.notice = Some(Notice::warning(format!(
                "{} changed on disk — R to take it, losing your edits",
                self.project.path().display()
            )));
            return;
        }

        self.reload_from_disk();
    }

    /// Whether somebody else's write is waiting to be taken.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        self.stale
    }

    /// Re-read the project from disk, discarding whatever is in memory.
    ///
    /// The deliberate act `R` performs, and what `poll_file` does on its own
    /// when there is nothing to lose.
    pub fn reload_from_disk(&mut self) {
        let path = self.project.path().to_path_buf();
        // Taken before the project is replaced, so the same rule that governs
        // an agent's tool call governs a text editor in another window.
        let before = self.variant_fingerprint();

        match Project::open(&path) {
            Ok(project) => {
                self.project = project;
                self.dirty = false;
                self.stale = false;
                self.confirm_quit = false;
                self.watcher.accept(&path);

                // The editor holds a copy of the prompt and the preview holds
                // a copy of the artwork. Both are now somebody else's.
                self.editor = Self::editor_for(
                    self.project
                        .metadata()
                        .prompt
                        .clone()
                        .unwrap_or_default()
                        .as_str(),
                );
                self.absorb_change(before);
                self.notice = Some(Notice::info(format!("reloaded {}", path.display())));
            }
            // Reported, not fatal. A half-written file is a normal thing to
            // catch mid-save, and the next tick will find it finished.
            Err(error) => {
                self.notice = Some(Notice::warning(format!("could not reload: {error}")));
            }
        }
    }

    /// Ask to leave the workspace.
    ///
    /// Unsaved work is worth one question and no more: the first request with
    /// changes pending asks, the second discards them.
    pub fn request_quit(&mut self) {
        if self.dirty && !self.confirm_quit {
            self.confirm_quit = true;
            return;
        }
        self.should_quit = true;
    }

    /// Withdraw a pending quit confirmation.
    ///
    /// Anything other than another quit counts as a change of mind, so the
    /// warning does not linger over an unrelated keypress.
    pub const fn cancel_quit_confirmation(&mut self) {
        self.confirm_quit = false;
    }

    /// Move the selection down within the focused pane.
    pub fn select_next(&mut self) {
        let len = self.focused_len();
        if len == 0 {
            return;
        }
        let index = self.focus_index();
        // Wrapping, because a four-item list is faster to cycle than to
        // realise you have hit the end of.
        self.selected[index] = (self.selected[index] + 1) % len;
    }

    /// Move the selection up within the focused pane.
    pub fn select_previous(&mut self) {
        let len = self.focused_len();
        if len == 0 {
            return;
        }
        let index = self.focus_index();
        self.selected[index] = (self.selected[index] + len - 1) % len;
    }

    /// The selected row in a given pane.
    #[must_use]
    pub fn selection(&self, focus: Focus) -> usize {
        let index = Focus::ALL
            .iter()
            .position(|candidate| *candidate == focus)
            .unwrap_or_default();
        // Clamped rather than trusted: metadata can shrink under a selection
        // once editing exists, and an out-of-range index would panic on draw.
        self.selected[index].min(self.pane_len(focus).saturating_sub(1))
    }

    /// How many rows a given pane has.
    fn pane_len(&self, focus: Focus) -> usize {
        match focus {
            // Prose, not rows. The arrows scroll it rather than selecting in
            // it, so it has nothing for the selection machinery to move.
            Focus::Prompt => 0,
            Focus::Palette => self.project.metadata().palette.len(),
        }
    }

    /// Which variant is on screen.
    ///
    /// In `Renders` mode that is the one the selected specification names,
    /// which is not necessarily the one with the same position in the list.
    #[must_use]
    pub fn current_variant(&self) -> Option<String> {
        let metadata = self.project.metadata();
        match self.mode {
            Mode::Variants => metadata
                .variants
                .get(self.tab())
                .map(|variant| variant.name.clone()),
            Mode::Renders => metadata
                .renders
                .get(self.tab())
                .map(|spec| spec.variant.clone()),
        }
    }

    /// What the preview should be showing.
    ///
    /// A render specification is the more specific statement: it names a
    /// variant *and* a size and background, so in that mode it is obeyed
    /// exactly. A variant on its own is previewed at a default size.
    #[must_use]
    pub fn preview_spec(&self) -> Option<RenderSpec> {
        let metadata = self.project.metadata();

        let mut spec = match self.mode {
            Mode::Renders => {
                let mut spec = metadata.renders.get(self.tab())?.clone();
                // A preview is pixels on a screen, so an SVG specification is
                // previewed by rasterising it. Handing SVG bytes to the preview
                // layer instead put the words "not a PNG" in the pane, which is
                // true, useless, and looks like a bug in the project.
                spec.format = Format::Png;
                spec
            }
            Mode::Variants => {
                let variant = metadata.variants.get(self.tab())?;
                RenderSpec::square(&variant.name, &variant.name, PREVIEW_SIZE)
            }
        };

        self.fit_to_pane(&mut spec);
        Some(spec)
    }

    /// Shrink a specification to something the pane can actually show.
    ///
    /// Previewing `social` at its declared 1280x640 rasterises 819 000 pixels
    /// to be drawn in a pane holding a few thousand — which is where the
    /// workspace's stutter came from.
    ///
    /// Halving, rather than scaling to fit exactly, for three reasons: the
    /// aspect ratio stays exact, so nothing letterboxes that would not have;
    /// the size changes in a handful of steps, so dragging the divider does
    /// not re-render on every mouse event; and a power-of-two reduction is the
    /// one resamplers are kindest to.
    ///
    /// Never enlarges. A preview must not invent detail the asset lacks.
    fn fit_to_pane(&self, spec: &mut RenderSpec) {
        let (box_width, box_height) = self.preview_pixels;
        if box_width == 0 || box_height == 0 {
            return;
        }

        while spec.width > box_width || spec.height > box_height {
            let (half_width, half_height) = (spec.width / 2, spec.height / 2);
            if half_width < MIN_PREVIEW_SIZE || half_height < MIN_PREVIEW_SIZE {
                break;
            }
            spec.width = half_width;
            spec.height = half_height;
        }
    }

    /// Throw the current preview away and render it again.
    ///
    /// Also hands the worker the project as it stands, so this is what picks
    /// up a change to the file rather than merely redrawing what was already
    /// rendered.
    pub fn invalidate_preview(&mut self) {
        self.worker.reload(&self.project);
        self.rendered_from = None;
        self.desired = None;
        self.in_flight = None;
    }

    /// What the variant on screen currently looks like, as bytes.
    ///
    /// [`Renderer::fingerprint`] for the current variant at a fixed size. Two
    /// of these either side of a change answer the only question the preview
    /// has — whether *this* variant moved — without the tools layer having to
    /// report what an agent touched, which it cannot: `write_svg` replaces the
    /// whole document.
    ///
    /// `None` when there is nothing to isolate, which compares equal to itself
    /// and so leaves the preview alone.
    #[must_use]
    pub fn variant_fingerprint(&self) -> Option<String> {
        let variant = self.current_variant()?;
        // A fixed size, not the fitted one: the canvas is in the isolated
        // document, and comparing at the pane's size would call resizing the
        // terminal a change to the artwork.
        let spec = RenderSpec::square(&variant, &variant, PREVIEW_SIZE);
        Renderer::new(&self.project, RenderOptions::default())
            .and_then(|renderer| renderer.fingerprint(&spec))
            .ok()
    }

    /// Take a change to the project, re-rendering only if it shows.
    ///
    /// `before` is what [`Self::variant_fingerprint`] said beforehand. The
    /// worker is always given the new project, so the source view and the next
    /// tab are current; the preview is only thrown away when the variant on
    /// screen is one of the things that moved. An agent editing the wordmark
    /// while the icon is on screen must not cost a rasterise and, under Kitty,
    /// a megabyte of terminal traffic.
    pub fn absorb_change(&mut self, before: Option<String>) {
        self.worker.reload(&self.project);
        if before != self.variant_fingerprint() {
            self.rendered_from = None;
            self.desired = None;
            self.in_flight = None;
        }
    }

    /// Whether a render is in progress.
    ///
    /// Drives the spinner, and the shorter poll interval that lets it animate.
    #[must_use]
    pub const fn is_rendering(&self) -> bool {
        self.in_flight.is_some()
    }

    /// How long the last completed render took.
    #[must_use]
    pub const fn last_render(&self) -> Option<Duration> {
        self.last_render
    }

    /// Draw everything except the image, for one frame.
    ///
    /// Used to get a spinner on screen before the blocking write that follows.
    pub const fn hold_image(&mut self) {
        self.holding_image = true;
    }

    /// Resume drawing the image.
    pub const fn release_image(&mut self) {
        self.holding_image = false;
    }

    /// Whether the image is being withheld for a frame.
    #[must_use]
    pub const fn is_holding_image(&self) -> bool {
        self.holding_image
    }

    /// When the work the preview is waiting on began, if it is waiting.
    ///
    /// The spinner itself lives in [`super::panes`], with the agent's: two
    /// implementations of the same animation drifted apart the moment one of
    /// them was tuned, and "generating a preview" and "the agent is working"
    /// are the same statement to the person reading them.
    #[must_use]
    pub fn rendering_since(&self) -> Option<Instant> {
        match (self.in_flight.as_ref(), self.holding_image) {
            (Some((_, _, started)), _) => Some(*started),
            // Nothing is rendering, but an image is about to be written and
            // that write is the slow part under tmux.
            (None, true) => Some(Instant::now()),
            (None, false) => None,
        }
    }

    /// Start a render if the selection has settled on something new.
    ///
    /// Never blocks. The work happens on [`Worker`]; this only decides whether
    /// to ask for it.
    pub fn update_preview(&mut self) {
        let Some(spec) = self.preview_spec() else {
            self.preview = Preview::Failed("this project declares no variants".to_owned());
            self.desired = None;
            return;
        };

        if self.desired.as_ref() != Some(&spec) {
            self.desired = Some(spec);
            self.changed_at = Instant::now();
        }

        // The debounce is the only thing separating "the selection moved" from
        // "the selection settled". Everything else is decided by `wanted`.
        if self.changed_at.elapsed() >= DEBOUNCE
            && let Some(spec) = self.wanted()
        {
            self.start_render(spec);
        }
    }

    /// Start a render at once, without waiting out the debounce.
    ///
    /// The debounce exists to absorb a stream of keypresses; a caller that has
    /// decided it wants a render now has nothing to absorb.
    ///
    /// Returns whether one was started.
    pub fn begin_render(&mut self) -> bool {
        self.update_preview();
        match self.wanted() {
            Some(spec) => {
                self.start_render(spec);
                true
            }
            None => false,
        }
    }

    /// The render that ought to be running, if it is not already.
    fn wanted(&self) -> Option<RenderSpec> {
        let desired = self.desired.as_ref()?;
        let on_screen = self.rendered_from.as_ref() == Some(desired);
        let under_way = self
            .in_flight
            .as_ref()
            .is_some_and(|(_, spec, _)| spec == desired);

        (!on_screen && !under_way).then(|| desired.clone())
    }

    /// Hand a specification to the worker.
    fn start_render(&mut self, spec: RenderSpec) {
        self.sequence = self.sequence.wrapping_add(1);
        if self.worker.request(self.sequence, spec.clone()) {
            self.in_flight = Some((self.sequence, spec, Instant::now()));
        } else {
            self.preview = Preview::Failed("the renderer stopped".to_owned());
        }
    }

    /// Take delivery of any finished render.
    ///
    /// Returns whether anything changed, so the caller can tell a frame worth
    /// drawing from one that is not.
    pub fn collect_preview(&mut self) -> bool {
        let mut changed = false;
        while let Some(rendered) = self.worker.collect() {
            changed |= self.apply(rendered);
        }
        changed
    }

    /// Show a finished render, if it is still the one being waited for.
    ///
    /// An answer to a superseded request is stale by definition, and showing
    /// it would flick the preview back to something the selection has already
    /// left.
    fn apply(&mut self, rendered: Rendered) -> bool {
        let Some((seq, spec, _)) = self.in_flight.as_ref() else {
            return false;
        };
        if *seq != rendered.seq {
            return false;
        }

        let spec = spec.clone();
        self.last_render = Some(rendered.elapsed);
        self.preview = match rendered.image {
            Ok(image) => Preview::Ready {
                image: Box::new(image),
                caption: format!(
                    "{} — {}x{} on {}",
                    spec.variant, spec.width, spec.height, spec.background
                ),
            },
            Err(reason) => Preview::Failed(reason),
        };
        self.generation = self.generation.wrapping_add(1);
        self.rendered_from = Some(spec);
        self.in_flight = None;
        true
    }

    /// Ask for a preview and wait for it.
    ///
    /// The blocking form, for the first frame and for tests. Bypasses the
    /// debounce, which exists to absorb a stream of keypresses and has nothing
    /// to absorb here.
    pub fn refresh_preview(&mut self) {
        self.begin_render();

        while self.is_rendering() {
            // Applied through the same path a frame uses, so the blocking form
            // cannot diverge from the asynchronous one.
            match self.worker.wait(Duration::from_secs(30)) {
                Some(rendered) => {
                    self.apply(rendered);
                }
                None => {
                    self.preview = Preview::Failed("the renderer stopped answering".to_owned());
                    self.in_flight = None;
                    return;
                }
            }
        }
    }

    /// Record where the preview was drawn, and how big that is in pixels.
    ///
    /// The pixel size is what [`App::preview_spec`] fits the render to. It
    /// comes from the backend, because only the backend knows the terminal's
    /// cell size.
    pub(crate) fn set_preview_area(&mut self, area: Rect, cell: (u16, u16), scale: Scale) {
        self.preview_area = area;
        // Scaled down here rather than at transmission time, so the renderer
        // produces fewer pixels in the first place instead of producing them
        // and throwing them away.
        self.preview_pixels = (
            scale.apply(u32::from(area.width) * u32::from(cell.0)),
            scale.apply(u32::from(area.height) * u32::from(cell.1)),
        );
    }

    /// Where the preview was drawn last frame.
    #[must_use]
    pub const fn preview_area(&self) -> Rect {
        self.preview_area
    }

    /// Record where a pane was drawn.
    pub(crate) fn set_area(&mut self, focus: Focus, area: Rect) {
        self.areas[Self::index_of(focus)] = area;
    }

    /// Where a pane was drawn last frame.
    #[must_use]
    pub fn area(&self, focus: Focus) -> Rect {
        self.areas[Self::index_of(focus)]
    }

    /// The list scroll state for a pane, synchronised to its selection.
    pub(crate) fn list_state(&mut self, focus: Focus) -> &mut ListState {
        let index = Self::index_of(focus);
        let len = self.pane_len(focus);
        let selected = self.selected[index].min(len.saturating_sub(1));
        let state = &mut self.list_states[index];
        // Kept in step here rather than at every point that moves the
        // selection: ratatui scrolls to whatever the state says, so the state
        // is the thing that has to be right at draw time.
        state.select((len > 0).then_some(selected));
        state
    }

    /// The pane containing a point, if any.
    #[must_use]
    pub fn pane_at(&self, column: u16, row: u16) -> Option<Focus> {
        Focus::ALL.into_iter().find(|focus| {
            self.area(*focus)
                .contains(ratatui::layout::Position::new(column, row))
        })
    }

    /// Select the row a click landed on.
    ///
    /// Returns whether anything was selected. A click on a pane's border or
    /// past the end of its list focuses the pane without moving the cursor,
    /// which is what makes clicking a title bar harmless.
    pub fn select_at(&mut self, focus: Focus, row: u16) -> bool {
        let area = self.area(focus);
        // One row of border at the top, and the list's own scroll offset.
        let Some(offset) = row.checked_sub(area.y + 1) else {
            return false;
        };
        let index = usize::from(offset) + self.list_states[Self::index_of(focus)].offset();
        if index >= self.pane_len(focus) {
            return false;
        }
        self.selected[Self::index_of(focus)] = index;
        true
    }

    /// Move the description column's edge, keeping both columns usable.
    pub fn resize_column(&mut self, column: u16, total_width: u16) {
        let most = total_width.saturating_sub(MIN_PREVIEW).max(MIN_COLUMN);
        self.column_width = column.clamp(MIN_COLUMN, most);
    }

    /// How wide the description column should be drawn.
    ///
    /// Half the body until somebody drags it. Half rather than a fixed number
    /// of characters because the left column is now prose and a palette rather
    /// than four lists of known width, and prose wants room in proportion to
    /// the screen it is being read on.
    pub fn column(&mut self, total_width: u16) -> u16 {
        // Zero is "nobody has said", not a width. A dragged column is never
        // zero, because `resize_column` clamps it to `MIN_COLUMN`.
        let wanted = if self.column_width == 0 {
            total_width / 2
        } else {
            self.column_width
        };
        self.resize_column(wanted, total_width);
        self.column_width
    }

    /// Record where a toolbar button was drawn.
    pub(crate) fn set_toolbar(&mut self, buttons: Vec<(Button, Rect)>) {
        self.toolbar = buttons;
    }

    /// The toolbar button at a point, if any.
    #[must_use]
    pub fn button_at(&self, column: u16, row: u16) -> Option<Button> {
        self.toolbar
            .iter()
            .find(|(_, area)| area.contains(ratatui::layout::Position::new(column, row)))
            .map(|(button, _)| *button)
    }

    /// Where a toolbar button was drawn last frame.
    #[must_use]
    pub fn button_area(&self, button: Button) -> Option<Rect> {
        self.toolbar
            .iter()
            .find(|(candidate, _)| *candidate == button)
            .map(|(_, area)| *area)
    }

    /// Record where the preview's tabs were drawn.
    pub(crate) fn set_tab_areas(&mut self, areas: Vec<Rect>) {
        self.tabs = areas;
    }

    /// The tab at a point, if any.
    #[must_use]
    pub fn tab_at(&self, column: u16, row: u16) -> Option<usize> {
        self.tabs
            .iter()
            .position(|area| area.contains(ratatui::layout::Position::new(column, row)))
    }

    /// The modal covering the workspace, if any.
    #[must_use]
    pub const fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    /// Open the render specifications editor.
    ///
    /// The one operation on the project that is neither a keystroke into a
    /// field nor an agent's doing: adding, removing and retyping a
    /// specification needs a table, and a table needs the whole screen.
    pub fn open_renders_editor(&mut self) {
        // The editor opens on what is on screen, so the row under the cursor
        // is the specification the tab bar was showing.
        let row = if self.mode == Mode::Renders {
            self.tab()
        } else {
            0
        };
        self.modal = Some(Modal::Renders(RendersEditor::new(row, &self.project)));
    }

    /// Close whatever modal is open.
    pub fn close_modal(&mut self) {
        self.modal = None;
    }

    /// Apply a keypress to the modal, which owns every key while it is open.
    ///
    /// Returns whether the modal is still open afterwards, so the caller does
    /// not have to ask twice.
    pub fn modal_key(&mut self, key: KeyEvent) {
        let Some(Modal::Renders(mut editor)) = self.modal.take() else {
            return;
        };
        if editor.key(key, &mut self.project) {
            self.dirty = true;
        }
        if !editor.closed() {
            self.modal = Some(Modal::Renders(editor));
        }
    }

    /// Note a click, reporting whether it is a repeat of the last one.
    ///
    /// "Same place, soon after" rather than a count: a double-click that moved
    /// to a different row is two deliberate selections, not one gesture.
    pub fn register_click(&mut self, focus: Focus, row: u16) -> bool {
        let now = Instant::now();
        let repeat = self.last_click.is_some_and(|(previous, at, when)| {
            previous == focus && at == row && now.duration_since(when) < DOUBLE_CLICK
        });
        // Cleared on a repeat, so a third click starts a new gesture rather
        // than firing the action again.
        self.last_click = (!repeat).then_some((focus, row, now));
        repeat
    }

    /// Note a click on a tab, reporting whether it is a repeat of the last one.
    ///
    /// Its own record rather than [`Self::register_click`]'s: the tabs are not
    /// a pane, and a click on a tab followed by a click on the same row of a
    /// pane is two gestures rather than a double-click.
    pub fn register_tab_click(&mut self, index: usize) -> bool {
        let now = Instant::now();
        let repeat = self
            .last_tab_click
            .is_some_and(|(at, when)| at == index && now.duration_since(when) < DOUBLE_CLICK);
        self.last_tab_click = (!repeat).then_some((index, now));
        repeat
    }

    /// Whether the prompt is what the keyboard would reach right now.
    ///
    /// `a` and `e` act on the prompt, and both would be silently useless from
    /// the palette or with the transcript in the box. A key that does nothing
    /// where it is offered is worse than one that is not offered.
    #[must_use]
    pub fn can_send(&self) -> bool {
        self.focus == Focus::Prompt && self.left == LeftView::Prompt
    }

    /// Begin dragging the column divider.
    pub const fn begin_resize(&mut self) {
        self.resizing = true;
    }

    /// Stop dragging the column divider.
    pub const fn end_resize(&mut self) {
        self.resizing = false;
    }

    /// Whether the column divider is being dragged.
    #[must_use]
    pub const fn is_resizing(&self) -> bool {
        self.resizing
    }

    /// The full width of the last frame.
    #[must_use]
    pub const fn total_width(&self) -> u16 {
        self.column_width + self.preview_area.width
    }

    /// Move the selection down in a named pane.
    pub fn select_next_in(&mut self, focus: Focus) {
        self.step(focus, 1);
    }

    /// Move the selection up in a named pane.
    pub fn select_previous_in(&mut self, focus: Focus) {
        self.step(focus, -1);
    }

    /// Move a pane's selection by one, wrapping.
    fn step(&mut self, focus: Focus, delta: isize) {
        let len = self.pane_len(focus);
        if len == 0 {
            return;
        }
        let index = Self::index_of(focus);
        let current = self.selected[index].min(len - 1);
        let next = (current as isize + delta).rem_euclid(len as isize);
        self.selected[index] = next as usize;
    }

    /// Render the specification on screen and write it to `dist/`.
    ///
    /// The result goes to the status line rather than being returned: an
    /// export that fails must not close the workspace, and a project mid-edit
    /// fails often.
    pub fn export_selected_render(&mut self) {
        let Some(spec) = self
            .project
            .metadata()
            .renders
            .get(self.tab())
            .filter(|_| self.mode == Mode::Renders)
            .cloned()
        else {
            return;
        };

        let directory = std::path::Path::new(EXPORT_DIRECTORY);
        self.notice = Some(
            match Renderer::new(&self.project, RenderOptions::default())
                .and_then(|renderer| renderer.render(&spec))
                .and_then(|asset| asset.write_to(directory))
            {
                Ok(path) => Notice::info(format!("wrote {}", path.display())),
                Err(error) => Notice::warning(format!("export failed: {error}")),
            },
        );
    }

    /// Which image the preview currently holds.
    ///
    /// Changes whenever the pixels do, and never otherwise.
    #[must_use]
    pub const fn preview_generation(&self) -> u64 {
        self.generation
    }

    /// The current preview.
    #[must_use]
    pub fn preview(&self) -> &Preview {
        &self.preview
    }

    /// The current preview's pixels, if it has any.
    #[must_use]
    pub fn preview_image(&self) -> Option<&Image> {
        match &self.preview {
            Preview::Ready { image, .. } => Some(image),
            _ => None,
        }
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

    /// Replace the editor's buffer, as `$EDITOR` returning would.
    fn set_buffer(app: &mut App, text: &str) {
        app.set_draft(text);
    }

    #[test]
    fn marking_a_project_as_new_makes_it_dirty_and_raises_a_notice() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("new.svg");

        let mut app = App::new(
            Project::init(&path).expect("the template is a valid project"),
            "blocks",
        );
        assert!(!app.is_dirty(), "App::new alone must not decide this");

        app.mark_as_new_project();

        assert!(app.is_dirty());
        assert!(app.notice.is_some());
    }

    #[test]
    fn the_prompt_editor_starts_from_what_the_project_says() {
        let app = app();
        assert_eq!(
            app.draft(),
            app.project.metadata().prompt.clone().unwrap_or_default()
        );
    }

    #[test]
    fn a_prompt_edited_to_nothing_removes_the_element_rather_than_writing_an_empty_one() {
        // An empty `<shaipe:prompt/>` is not the same document as no prompt at
        // all, and only one of the two round-trips.
        let mut app = app();
        set_buffer(&mut app, "");
        app.commit_prompt();

        assert_eq!(app.project.metadata().prompt, None);
        assert!(
            !app.project
                .to_svg()
                .expect("serialisable")
                .contains("prompt")
        );
    }

    #[test]
    fn a_committed_prompt_is_trimmed_so_it_survives_a_round_trip() {
        // The reader trims, so anything else would come back different from
        // what was written.
        let mut app = app();
        set_buffer(&mut app, "  spaced out  ");
        app.commit_prompt();

        assert_eq!(app.project.metadata().prompt.as_deref(), Some("spaced out"));
    }

    #[test]
    fn committing_the_prompt_unchanged_leaves_the_project_clean() {
        let mut app = app();
        app.commit_prompt();

        assert!(!app.is_dirty());
    }

    #[test]
    fn saving_the_workspace_writes_the_project_and_clears_the_dirty_marker() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).expect("the fixture is writable");

        let mut app = App::new(
            Project::open(&path).expect("the fixture is a project"),
            "blocks",
        );
        set_buffer(&mut app, "a rewritten prompt");
        app.commit_prompt();
        assert!(app.is_dirty());

        app.save();

        assert!(!app.is_dirty());
        assert!(
            std::fs::read_to_string(&path)
                .expect("the project was written")
                .contains("a rewritten prompt")
        );
    }

    #[test]
    fn a_failed_save_reports_itself_without_closing_the_workspace() {
        // The workspace is the only place the unsaved work still exists.
        let mut app = App::new(
            Project::from_source("no/such/directory/logo.svg", fixtures::PROJECT.to_owned())
                .expect("the fixture is a project"),
            "blocks",
        );
        set_buffer(&mut app, "a prompt that cannot be written");
        app.commit_prompt();

        app.save();

        assert!(app.is_dirty(), "nothing reached the disk");
        assert!(!app.should_quit);
        assert!(
            app.notice
                .as_ref()
                .and_then(Notice::text)
                .is_some_and(|notice| notice.contains("save failed")),
            "{:?}",
            app.notice
        );
    }

    #[test]
    fn each_mode_remembers_its_own_tab() {
        // Otherwise looking at a render specification and coming back would
        // land on the first variant rather than the one that was on screen.
        let mut app = app();

        app.next_tab();
        assert_eq!(app.tab(), 1);

        app.toggle_mode();
        assert_eq!(app.tab(), 0, "the specifications start at their own first");

        app.toggle_mode();
        assert_eq!(app.tab(), 1, "the variant that was on screen came back");
    }

    #[test]
    fn the_tabs_wrap_at_both_ends() {
        // A project has a handful of variants, and reaching the last by going
        // forwards is quicker than noticing that the end has been reached.
        let mut app = app();

        app.previous_tab();
        assert_eq!(app.tab(), 1, "moving left from the first wraps to the last");
        app.next_tab();
        assert_eq!(
            app.tab(),
            0,
            "moving right from the last wraps to the first"
        );
    }

    #[test]
    fn the_palette_selection_wraps_at_both_ends() {
        let mut app = app();
        app.focus = Focus::Palette;

        app.select_previous();
        assert_eq!(app.selection(Focus::Palette), 1);
        app.select_next();
        assert_eq!(app.selection(Focus::Palette), 0);
    }

    #[test]
    fn moving_the_selection_in_a_pane_with_no_rows_does_nothing() {
        let mut app = app();
        app.focus = Focus::Prompt;
        app.select_next();
        assert_eq!(app.selection(Focus::Prompt), 0);
    }

    #[test]
    fn a_tab_left_beyond_the_end_of_a_shrunken_list_is_clamped() {
        // An agent's edit can delete the variant that was on screen, and an
        // unclamped index would panic during drawing, taking the terminal
        // down with it.
        let mut app = app();
        app.next_tab();
        app.project.metadata_mut().variants.truncate(1);

        assert_eq!(app.tab(), 0);
    }

    #[test]
    fn a_render_specifications_tab_previews_that_specification_exactly() {
        let mut app = app();
        app.toggle_mode();
        app.next_tab();

        let spec = app.preview_spec().unwrap();
        assert_eq!(spec.name, "banner");
        assert_eq!((spec.width, spec.height), (128, 32));
    }

    #[test]
    fn a_variants_tab_previews_it_at_a_default_size() {
        let app = app();
        assert_eq!(app.preview_spec().unwrap().variant, "icon");
    }

    #[test]
    fn the_preview_is_rendered_once_and_then_cached() {
        let mut app = app();
        app.refresh_preview();
        assert!(matches!(app.preview(), Preview::Ready { .. }));

        // A second refresh with nothing moved must not re-rasterise.
        let before = app.preview().clone();
        app.refresh_preview();
        assert_eq!(app.preview(), &before);
    }

    #[test]
    fn moving_between_tabs_makes_the_preview_follow() {
        let mut app = app();
        app.toggle_mode();
        app.refresh_preview();
        let Preview::Ready { image, .. } = app.preview().clone() else {
            panic!("expected a preview");
        };
        assert_eq!((image.width(), image.height()), (32, 32));

        app.next_tab();
        app.refresh_preview();
        let Preview::Ready { image, .. } = app.preview().clone() else {
            panic!("expected a preview");
        };
        assert_eq!((image.width(), image.height()), (128, 32));
    }

    /// Pretend a pane of `cells` at a 10x20 cell size has been drawn.
    fn with_pane(app: &mut App, columns: u16, rows: u16) {
        app.set_preview_area(Rect::new(0, 0, columns, rows), (10, 20), Scale::FULL);
    }

    #[test]
    fn a_preview_is_halved_until_it_fits_the_pane() {
        // `social` is 1280x640. In a 58x27 pane at 10x20 px per cell — 580x540
        // — it must not be rasterised at its declared size, which is 819 000
        // pixels for a few thousand cells' worth of screen.
        let mut app = app();
        app.project.metadata_mut().renders.clear();
        app.project.metadata_mut().renders.push(RenderSpec {
            name: "social".to_owned(),
            variant: "icon".to_owned(),
            width: 1280,
            height: 640,
            format: Format::Png,
            background: crate::project::Background::Transparent,
        });
        app.mode = Mode::Renders;
        with_pane(&mut app, 58, 27);

        let spec = app.preview_spec().unwrap();
        assert_eq!((spec.width, spec.height), (320, 160));
    }

    #[test]
    fn fitting_a_preview_preserves_its_aspect_ratio_exactly() {
        // Halving is used precisely so the ratio survives. Scaling each side
        // to fit independently would letterbox an asset that does not.
        let mut app = app();
        app.project.metadata_mut().renders.clear();
        app.project.metadata_mut().renders.push(RenderSpec {
            name: "wide".to_owned(),
            variant: "icon".to_owned(),
            width: 1600,
            height: 400,
            format: Format::Png,
            background: crate::project::Background::Transparent,
        });
        app.mode = Mode::Renders;
        with_pane(&mut app, 30, 10);

        let spec = app.preview_spec().unwrap();
        assert_eq!(spec.width / spec.height, 4, "1600x400 is 4:1");
        assert!(spec.width <= 300, "should have shrunk: {}", spec.width);
    }

    #[test]
    fn a_preview_is_never_enlarged_to_fill_the_pane() {
        // A preview must not invent detail the asset does not have.
        let mut app = app();
        with_pane(&mut app, 200, 60);

        let spec = app.preview_spec().unwrap();
        assert_eq!((spec.width, spec.height), (PREVIEW_SIZE, PREVIEW_SIZE));
    }

    #[test]
    fn a_preview_stops_shrinking_before_it_becomes_unrecognisable() {
        let mut app = app();
        with_pane(&mut app, 1, 1);

        let spec = app.preview_spec().unwrap();
        assert!(spec.width >= MIN_PREVIEW_SIZE, "shrank to {}", spec.width);
    }

    #[test]
    fn before_the_first_frame_a_preview_uses_its_declared_size() {
        // `preview_pixels` is `(0, 0)` until something has been drawn, and a
        // zero-sized pane must not be read as "shrink to nothing".
        let app = app();
        assert_eq!(app.preview_spec().unwrap().width, PREVIEW_SIZE);
    }

    #[test]
    fn resizing_the_pane_eventually_re_renders_the_preview() {
        // The fitted size is part of the cache key, so growing the pane has to
        // produce a better preview rather than a stretched old one.
        let mut app = app();
        with_pane(&mut app, 10, 5);
        app.refresh_preview();
        let small = app.preview_image().unwrap().width();

        with_pane(&mut app, 80, 40);
        app.refresh_preview();
        let large = app.preview_image().unwrap().width();

        assert!(large > small, "{small} then {large}");
    }

    #[test]
    fn an_svg_render_specification_is_previewed_by_rasterising_it() {
        // Otherwise the pane reads "not a PNG", which is true and useless.
        let mut app = app();
        app.project.metadata_mut().renders.clear();
        let mut spec = RenderSpec::square("vector", "icon", 64);
        spec.format = Format::Svg;
        app.project.metadata_mut().renders.push(spec);

        app.mode = Mode::Renders;
        assert_eq!(app.preview_spec().unwrap().format, Format::Png);

        app.invalidate_preview();
        app.refresh_preview();
        assert!(matches!(app.preview(), Preview::Ready { .. }));
    }

    #[test]
    fn a_render_in_progress_is_something_the_spinner_can_be_derived_from() {
        // `Preview::Pending` and the spinner were unreachable while rendering
        // was synchronous: the frame could only be drawn once the render had
        // already finished, so there was never anything to report.
        let mut app = app();
        assert!(app.rendering_since().is_none(), "nothing is rendering yet");

        app.begin_render();

        assert!(app.is_rendering());
        assert!(
            app.rendering_since().is_some(),
            "a running render must be visible"
        );

        app.refresh_preview();
        assert!(!app.is_rendering());
        assert!(app.rendering_since().is_none());
    }

    #[test]
    fn a_selection_that_is_still_moving_does_not_start_a_render() {
        // Arrowing through a list must render the entry you stop on, not every
        // entry you pass over — each one costs a rasterise and, under Kitty, a
        // megabyte of terminal traffic.
        let mut app = app();
        app.refresh_preview();
        app.mode = Mode::Renders;

        app.next_tab();
        app.update_preview();
        assert!(
            !app.is_rendering(),
            "a render started before the selection settled"
        );

        app.next_tab();
        app.update_preview();
        assert!(!app.is_rendering());

        // Once it settles, it renders.
        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert!(app.is_rendering());
    }

    #[test]
    fn a_burst_of_keys_costs_one_render_not_one_per_key() {
        // The event loop drains everything that has arrived before drawing, so
        // three queued arrow keys reach this as three selection moves and one
        // decision. Under Kitty each render is over a megabyte of terminal
        // traffic, so the difference is three megabytes nobody would see.
        let mut app = app();
        app.refresh_preview();
        let after_initial = app.sequence;

        for _ in 0..3 {
            app.next_tab();
        }

        app.update_preview();
        assert_eq!(
            app.sequence, after_initial,
            "nothing should start while the selection is still moving"
        );

        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert_eq!(
            app.sequence,
            after_initial + 1,
            "the whole burst should cost exactly one render"
        );
    }

    #[test]
    fn returning_to_what_is_already_shown_costs_nothing() {
        // Arrowing down and back up again lands on the image already on
        // screen. Re-rendering it would be a megabyte for no change at all.
        let mut app = app();
        app.refresh_preview();
        let after_initial = app.sequence;

        app.next_tab();
        app.previous_tab();

        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert_eq!(app.sequence, after_initial);
        assert!(!app.is_rendering());
    }

    #[test]
    fn a_stale_answer_is_discarded_rather_than_shown() {
        // The worker answers requests in order, but a request can be
        // superseded while it is running. Showing its answer would flick the
        // preview back to something the selection has already left.
        let mut app = app();
        app.refresh_preview();
        let before = app.preview().clone();

        let stale = Rendered {
            seq: app.sequence.wrapping_add(999),
            image: Ok(Image::from_rgba(1, 1, vec![1, 2, 3, 4])),
            elapsed: Duration::ZERO,
        };
        assert!(!app.apply(stale));
        assert_eq!(app.preview(), &before);
    }

    #[test]
    fn a_project_that_cannot_be_rendered_shows_the_reason_instead_of_quitting() {
        // A project mid-edit is often broken. Taking the workspace down on
        // every intermediate state would make it useless for editing.
        let mut app = app();
        app.project.metadata_mut().variants.clear();
        app.invalidate_preview();
        app.refresh_preview();

        assert!(matches!(app.preview(), Preview::Failed(_)));
    }
    /// Rewrite the project's source, as `write_svg` does.
    ///
    /// Through `from_source` rather than by poking at the metadata, because
    /// replacing the whole document is exactly what the only mutating tool
    /// there is does — and the reason nothing can say which variant moved.
    fn rewrite(app: &mut App, from: &str, to: &str) {
        app.project = Project::from_source("logo.svg", app.project.source().replace(from, to))
            .expect("still a project");
    }

    #[test]
    fn an_edit_to_another_variant_does_not_regenerate_the_preview() {
        // An agent rewriting the wordmark while the icon is on screen must not
        // cost a rasterise and, under Kitty, a megabyte of terminal traffic.
        let mut app = app();
        app.refresh_preview();
        let after_initial = app.sequence;
        assert_eq!(app.current_variant().as_deref(), Some("icon"));

        let before = app.variant_fingerprint();
        rewrite(
            &mut app,
            r#"<rect width="256" height="64""#,
            r#"<rect width="256" height="60""#,
        );
        app.absorb_change(before);

        // Twice, with the debounce wound back in between: the first call is
        // what settles on a specification, and backdating before it would be
        // undone by it.
        app.update_preview();
        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert_eq!(
            app.sequence, after_initial,
            "the wordmark moved, and the icon was re-rendered for it"
        );
    }

    #[test]
    fn an_edit_to_the_previewed_variant_regenerates_it() {
        // The other half of the same rule: the gate must not be so tight that
        // the preview stops following the agent at all.
        let mut app = app();
        app.refresh_preview();
        let after_initial = app.sequence;

        let before = app.variant_fingerprint();
        rewrite(
            &mut app,
            r#"<rect width="64" height="64""#,
            r#"<rect width="64" height="60""#,
        );
        app.absorb_change(before);

        app.update_preview();
        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert_eq!(app.sequence, after_initial + 1, "the preview went stale");
    }

    #[test]
    fn a_palette_colour_is_edited_in_place_and_reaches_the_document() {
        // No tool and no dialogue: the palette is a pane, and a colour is two
        // fields on the row the cursor is on.
        let mut app = app();
        app.focus = Focus::Palette;
        app.engage_editor();

        for _ in 0..7 {
            app.edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        for character in "#0066ff".chars() {
            app.edit_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        assert_eq!(
            app.project.metadata().palette.colours()[0]
                .value
                .to_string(),
            "#0066ff"
        );
        assert!(app.is_dirty());
        assert!(app.project.to_svg().unwrap().contains("#0066ff"));
    }

    #[test]
    fn editing_a_palette_colours_value_in_the_tui_restyles_bound_artwork() {
        // The same keystroke path as above, this time on a project where an
        // element is actually bound to the colour being typed — proving the
        // workspace's own editor restyles the mark, not only the tool does.
        const BOUND: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 64 64">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:palette>
        <shaipe:color name="accent" value="#f05032" role="accent"/>
      </shaipe:palette>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
      </shaipe:variants>
    </shaipe:project>
  </metadata>
  <symbol id="icon" viewBox="0 0 64 64"><rect width="64" height="64" fill="#f05032" shaipe:fill="accent"/></symbol>
  <use href="#icon" width="64" height="64"/>
</svg>
"##;

        let mut app = App::new(
            Project::from_source("logo.svg", BOUND.to_owned()).unwrap(),
            "blocks",
        );
        app.focus = Focus::Palette;
        app.engage_editor();

        for _ in 0..7 {
            app.edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        for character in "#0066ff".chars() {
            app.edit_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        assert!(app.project.source().contains(r##"fill="#0066ff""##));
        assert!(!app.project.source().contains(r##"fill="#f05032""##));
    }

    #[test]
    fn a_half_typed_colour_is_kept_on_screen_and_out_of_the_document() {
        // `#00` is what every hex colour looks like partway through being
        // typed. Refusing the keystroke would make the field unusable; letting
        // it through would put something that is not a colour into the project.
        let mut app = app();
        app.focus = Focus::Palette;
        app.engage_editor();

        for _ in 0..7 {
            app.edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        for character in "#00".chars() {
            app.edit_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        let edit = app.palette_edit().expect("the palette is being edited");
        assert_eq!(edit.text, "#00", "the keystrokes are invisible");
        assert!(!edit.valid, "and nothing says they are not a colour yet");

        // The document holds whatever the last thing that *was* a colour said
        // — the field commits as it is typed — and never the half of one.
        let committed = app.project.metadata().palette.colours()[0].value;
        assert!(
            committed.to_string().parse::<Rgba>().is_ok(),
            "{committed} is not a colour"
        );
        assert!(!app.project.to_svg().unwrap().contains("\"#00\""));
    }

    #[test]
    fn renaming_a_colour_keeps_finding_the_one_being_renamed() {
        // Looked up by name, the row would stop being found on the very first
        // keystroke — which is why the palette is edited by position.
        let mut app = app();
        app.focus = Focus::Palette;
        app.engage_editor();
        app.edit_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));

        for _ in 0..6 {
            app.edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        for character in "brand".chars() {
            app.edit_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        assert_eq!(app.project.metadata().palette.colours()[0].name, "brand");
    }

    #[test]
    fn a_colour_cannot_be_left_with_no_name_at_all() {
        // Nothing could refer to it afterwards, and the reader would not give
        // it back. The name commits as it is typed, so what this asserts is
        // that the *empty* buffer is the one thing never written.
        let mut app = app();
        app.focus = Focus::Palette;
        app.engage_editor();
        app.edit_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));

        for _ in 0..10 {
            app.edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        app.edit_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(
            !app.project.metadata().palette.colours()[0].name.is_empty(),
            "a colour was left with no name"
        );
    }

    /// A project in a temporary directory, so the tests can write to it.
    fn on_disk() -> (tempfile::TempDir, App) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).expect("the fixture is writable");
        let project = Project::open(&path).expect("it opens");
        (directory, App::new(project, "blocks"))
    }

    #[test]
    fn a_change_on_disk_is_taken_when_there_is_nothing_to_lose() {
        // The reason this exists: nothing over ACP can stop an agent reaching
        // for its own editor, so the workspace has to notice when one does.
        let (directory, mut app) = on_disk();
        let path = app.project.path().to_path_buf();

        std::fs::write(&path, fixtures::PROJECT.replace("#f05032", "#0066ff")).unwrap();
        app.poll_file();

        assert_eq!(
            app.project.metadata().palette.colours()[0]
                .value
                .to_string(),
            "#0066ff",
            "somebody else's write was not picked up"
        );
        assert!(!app.is_dirty());
        assert!(!app.is_stale());
        drop(directory);
    }

    #[test]
    fn a_change_on_disk_does_not_discard_unsaved_work() {
        // Both versions are somebody's. Choosing between them is a decision,
        // not a default.
        let (directory, mut app) = on_disk();
        let path = app.project.path().to_path_buf();

        app.engage_editor();
        app.set_draft("a wordless circular mark");
        assert!(app.is_dirty());

        std::fs::write(&path, fixtures::PROJECT.replace("#f05032", "#0066ff")).unwrap();
        app.poll_file();

        assert!(app.is_stale());
        assert_eq!(
            app.project.metadata().prompt.as_deref(),
            Some("a wordless circular mark"),
            "the unsaved prompt was thrown away"
        );
        assert!(
            app.notice
                .as_ref()
                .and_then(Notice::text)
                .is_some_and(|n| n.contains("changed on disk"))
        );
        drop(directory);
    }

    #[test]
    fn saving_over_somebody_elses_write_is_refused_once() {
        // Saving here would overwrite their work with bytes read before they
        // wrote. Refused once, then allowed, so there is a way through.
        let (directory, mut app) = on_disk();
        let path = app.project.path().to_path_buf();

        app.engage_editor();
        app.set_draft("mine");
        std::fs::write(&path, fixtures::PROJECT.replace("#f05032", "#0066ff")).unwrap();
        app.poll_file();

        app.save();
        assert!(app.is_dirty(), "the save should not have gone through");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().contains("#0066ff"),
            true,
            "their write was overwritten"
        );

        app.save();
        assert!(!app.is_dirty(), "a second save should go through");
        assert!(std::fs::read_to_string(&path).unwrap().contains("mine"));
        drop(directory);
    }

    #[test]
    fn the_workspaces_own_save_is_not_somebody_elses_change() {
        // Otherwise every `ctrl-s` would immediately report the file as
        // modified underneath, and refuse the next one.
        let (directory, mut app) = on_disk();

        app.engage_editor();
        app.set_draft("mine");
        app.save();
        app.poll_file();

        assert!(!app.is_stale());
        assert!(!app.is_dirty());
        drop(directory);
    }

    #[test]
    fn reloading_takes_the_file_and_forgets_the_editors_copy() {
        // The editor holds a copy of the prompt. Reloading without replacing
        // it would write the abandoned one back on the next keystroke.
        let (directory, mut app) = on_disk();
        let path = app.project.path().to_path_buf();

        app.engage_editor();
        app.set_draft("mine");

        std::fs::write(
            &path,
            fixtures::PROJECT.replace("A square and a bar.", "theirs"),
        )
        .unwrap();

        app.reload_from_disk();

        assert_eq!(app.draft(), "theirs");
        assert_eq!(app.project.metadata().prompt.as_deref(), Some("theirs"));
        assert!(!app.is_dirty());
        drop(directory);
    }

    #[test]
    fn a_file_that_cannot_be_reloaded_reports_it_rather_than_quitting() {
        // Catching a half-written file mid-save is normal.
        let (directory, mut app) = on_disk();
        let path = app.project.path().to_path_buf();
        std::fs::write(&path, "<svg truncated").unwrap();

        app.reload_from_disk();

        assert!(
            app.notice
                .as_ref()
                .and_then(Notice::text)
                .is_some_and(|n| n.contains("could not reload"))
        );
        assert!(!app.should_quit);
        // And the project in memory is untouched, so nothing was lost.
        assert_eq!(app.project.source(), fixtures::PROJECT);
        drop(directory);
    }
}
