//! The widgets each pane draws.
//!
//! One function per pane, each taking the state it needs and nothing more.
//! They build widgets and return them; where those widgets go is [`super::ui`]'s
//! decision, so a change to the layout does not touch a pane and a change to a
//! pane does not touch the layout.

use std::time::Instant;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};

use crate::project::{Palette, Rgba};

use super::app::{AgentStatus, App, Focus, LeftView, PaletteEdit, PaletteField, View};
use super::markdown;
use super::transcript::Entry;
use crate::acp::ToolStatus;

/// The border for a pane, highlighted when it has the keyboard.
pub fn frame(title: Focus, focused: bool) -> Block<'static> {
    frame_titled(title, focused, title.title(), "")
}

/// The border for a pane, with a title of its own and a note after it.
///
/// The prompt pane's title is not fixed: it says whether the editor is writing
/// the project's prompt or a message to an agent, because the two look
/// identical and do very different things.
pub fn frame_titled(title: Focus, focused: bool, name: &str, note: &str) -> Block<'static> {
    let style = if focused {
        Style::default().fg(Color::LightMagenta)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let _ = title;

    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style)
        .title(Span::styled(
            format!(" {name}{note} "),
            style.add_modifier(Modifier::BOLD),
        ))
}

/// The style a selected row is drawn in.
///
/// Applied by ratatui through `List::highlight_style`, which also scrolls the
/// pane to keep the selection visible — the reason the panes no longer style
/// the selected row themselves.
///
/// Only the pane holding the keyboard shows a highlight, so it is always
/// unambiguous which selection an arrow key will move.
fn selected(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

/// The prompt pane.
pub fn prompt(app: &App) -> Paragraph<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    match &app.project.metadata().prompt {
        // One `Line` per paragraph. Pushed whole, its newlines were not line
        // breaks in the `Vec<Line>` model at all — which is how a fifteen-row
        // prompt came to be counted as a single line.
        Some(prompt) => lines.extend(
            prompt
                .lines()
                .map(|line| Line::styled(line.to_owned(), Style::default().fg(Color::Gray))),
        ),
        None => lines.push(Line::styled(
            "No prompt recorded.",
            Style::default().fg(Color::DarkGray),
        )),
    }

    // Why the agent cannot be asked, beside the key that will not work.
    if let AgentStatus::Absent { reason } = &app.agent {
        lines.push(Line::raw(""));
        for line in reason.lines() {
            lines.push(Line::styled(
                line.to_owned(),
                Style::default().fg(Color::Yellow),
            ));
        }
    }

    // Read from the top, and not scrolled. A prompt is a thing you wrote, not
    // a conversation with a newest end — and the transcript that wanted the
    // bottom of this pane lives in the right-hand column now, where there is
    // room for it.
    Paragraph::new(lines).wrap(Wrap { trim: false })
}

/// A one-line summary of the agent, for the pane's title.
#[must_use]
pub fn agent_title(app: &App) -> String {
    // The key named here has to be the one that works from where the user is.
    // While the editor has the keyboard it swallows every key, so bare `a`
    // types a letter.
    let chord = if app.is_editing() { "alt+a" } else { "a" };

    match &app.agent {
        // The pane's body already says why, at length and in yellow. Naming a
        // key that would answer "no agent is connected" adds nothing.
        AgentStatus::Absent { .. } => String::new(),
        // Distinct from `Ready`, which it was not: a handshaking agent looked
        // exactly like one waiting for work, and that is the state in which a
        // prompt sits queued rather than running.
        AgentStatus::Connecting => "  starting the agent…".to_owned(),
        AgentStatus::Ready => format!("  {chord} to send it to the agent"),
        AgentStatus::Busy => "  working…".to_owned(),
    }
}

/// One turn of the spinner, from when the work started.
///
/// Derived from elapsed time rather than from a counter incremented per frame,
/// so it turns at a steady rate however often the workspace happens to redraw.
///
/// One implementation, deliberately. There were two — one for rasterising a
/// preview and one for the agent — and they were the same animation with the
/// same period written twice. To the person reading them they are the same
/// statement: something is happening and it has not finished.
fn spin(since: Instant) -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    const PERIOD: u128 = 90;

    let step = since.elapsed().as_millis() / PERIOD;
    FRAMES[(step as usize) % FRAMES.len()]
}

/// What the preview is waiting on, for its pane title.
#[must_use]
pub fn rendering(app: &App) -> Option<String> {
    app.rendering_since()
        .map(|since| format!("{} rendering", spin(since)))
}

/// What the agent is doing, for the right-hand end of the status line.
///
/// Without this the only sign that a prompt had been sent was the transcript
/// filling in, which happens seconds later and off to the left.
fn agent_activity(app: &App) -> Option<String> {
    let doing = match &app.agent {
        AgentStatus::Connecting => "starting…".to_owned(),
        AgentStatus::Busy => app
            .transcript
            .running_tool()
            // The tool it is on, when it is on one. A name is worth much more
            // than "working": it is the difference between knowing it is
            // rendering and wondering whether it has hung.
            .map_or_else(|| "working…".to_owned(), str::to_owned),
        AgentStatus::Ready | AgentStatus::Absent { .. } => return None,
    };

    Some(format!("{} {doing}", spin(app.agent_since())))
}

/// What the agent has been doing, one entry at a time.
///
/// Rendered into the top-left box, in place of the prompt — see
/// [`crate::tui::app::LeftView`].
pub fn transcript_lines(app: &App) -> Vec<Line<'static>> {
    if app.transcript.is_empty() {
        return vec![Line::styled(
            "Nothing yet. Press `a` on the prompt to ask the agent to \
             make the artwork match it.",
            Style::default().fg(Color::DarkGray),
        )];
    }

    transcript(app)
}

/// The entries themselves.
fn transcript(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for entry in app.transcript.entries() {
        match entry {
            Entry::You(text) => lines.extend(paragraphs(
                text,
                "you  ",
                Style::default().fg(Color::LightMagenta),
                Style::default(),
            )),
            Entry::Agent(text) => {
                lines.extend(paragraphs(
                    text,
                    "     ",
                    Style::default(),
                    Style::default(),
                ));
            }
            // Dimmed and marked, because reasoning read as a statement is how
            // a person ends up believing the agent said something it did not.
            Entry::Thought(text) => lines.extend(paragraphs(
                text,
                "     ",
                Style::default(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            Entry::Tool { title, status, .. } => lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", status.glyph()),
                    Style::default().fg(match status {
                        ToolStatus::Running => Color::Yellow,
                        ToolStatus::Completed => Color::Green,
                        ToolStatus::Failed => Color::Red,
                    }),
                ),
                Span::styled(title.clone(), Style::default().fg(Color::Cyan)),
            ])),
            Entry::Notice(text) => lines.extend(paragraphs(
                text,
                "     ",
                Style::default(),
                Style::default().fg(Color::Yellow),
            )),
        }
    }

    lines
}

/// One `Line` per rendered markdown row, with a gutter on the first.
///
/// `markdown::render` is what turns an agent's `**bold**` and `- item` into
/// actual emphasis and a real bullet; this function's own job is only the
/// gutter — the same one it always did. An agent's message contains
/// newlines, and a `Span` holding one is a single `Line` that draws as
/// several rows — which is how anything measured in lines ends up short.
/// Splitting here keeps the count and the drawing in agreement, and it is the
/// same mistake the prompt pane made with the project's prompt.
fn paragraphs(
    text: &str,
    gutter: &'static str,
    gutter_style: Style,
    style: Style,
) -> Vec<Line<'static>> {
    markdown::render(text, style)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let mut spans = vec![
                // The gutter only on the first, so a wrapped paragraph is not
                // mistaken for several entries.
                Span::styled(
                    if index == 0 { gutter } else { "     " },
                    if index == 0 {
                        gutter_style
                    } else {
                        Style::default()
                    },
                ),
            ];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// A colour swatch, drawn as two solid cells.
///
/// Two rather than one because a single cell reads as a typo next to text,
/// and because a swatch is the only thing in the pane that is not a word.
fn swatch(colour: Rgba) -> Span<'static> {
    Span::styled(
        "  ",
        Style::default().bg(Color::Rgb(colour.r, colour.g, colour.b)),
    )
}

/// The palette pane.
///
/// The one list left in the left column, and the only one that is edited in
/// place. `edit` is what has been typed rather than what the project holds:
/// a hex colour does not parse until its last character, so drawing the
/// committed value would make every keystroke but the final one invisible.
pub fn palette(palette: &Palette, focused: bool, edit: Option<PaletteEdit<'_>>) -> List<'static> {
    let items: Vec<ListItem> = palette
        .colours()
        .iter()
        .enumerate()
        .map(|(row, colour)| {
            let editing = edit.filter(|edit| edit.row == row);
            let field = |which: PaletteField, committed: String| match editing {
                Some(edit) if edit.field == which => Span::styled(
                    edit.text.to_owned(),
                    if edit.valid {
                        Style::default().fg(Color::Black).bg(Color::LightCyan)
                    } else {
                        // Shown, not refused: it is on its way to being a
                        // colour, and nothing has reached the document.
                        Style::default().fg(Color::Black).bg(Color::LightRed)
                    },
                ),
                _ => Span::raw(committed),
            };

            let mut spans = vec![
                swatch(colour.value),
                Span::raw(" "),
                field(PaletteField::Name, colour.name.clone()),
                Span::raw("  "),
                match editing {
                    Some(edit) if edit.field == PaletteField::Value => {
                        field(PaletteField::Value, String::new())
                    }
                    _ => Span::styled(
                        colour.value.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                },
            ];
            if let Some(role) = &colour.role {
                spans.push(Span::styled(
                    format!("  {role}"),
                    Style::default().fg(Color::Blue),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    List::new(items).highlight_style(selected(focused))
}

/// The key hints along the bottom.
/// One key hint, and how readily it is given up when the line is narrow.
///
/// The status line is the only place most of these keys are discoverable, so
/// which ones survive a small terminal is a decision rather than an accident.
/// It used to be neither: the line was exactly eighty columns, and `a` — the
/// way to reach the agent at all — was left out to keep it that way. A hint
/// that is not there is a key that does not exist.
#[derive(Debug, Clone, Copy)]
struct Hint {
    key: &'static str,
    action: &'static str,
    /// Higher goes first when there is not enough room. `0` never goes.
    expendable: u8,
}

impl Hint {
    /// A hint that is always shown, however narrow the terminal.
    const fn essential(key: &'static str, action: &'static str) -> Self {
        Self {
            key,
            action,
            expendable: 0,
        }
    }

    /// A hint that is given up when the line will not fit, most eager first.
    const fn optional(key: &'static str, action: &'static str, expendable: u8) -> Self {
        Self {
            key,
            action,
            expendable,
        }
    }

    /// How many columns it draws in.
    const fn width(&self) -> usize {
        // `key`, a space, `action`, and two of separator.
        self.key.len() + 1 + self.action.len() + 2
    }

    /// The spans it draws as.
    fn spans(&self) -> [Span<'static>; 2] {
        [
            Span::styled(
                self.key,
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}  ", self.action),
                Style::default().fg(Color::DarkGray),
            ),
        ]
    }
}

/// Drop hints until what remains fits, most expendable first.
///
/// Display order is preserved, so the line reads the same at every width — it
/// just says less. Dropping from the end instead would have thrown away `quit`
/// before `$EDITOR`, which is the wrong way round.
fn fit(hints: &[Hint], mut room: usize) -> Vec<Hint> {
    let mut kept: Vec<Hint> = hints.to_vec();

    while kept.iter().map(Hint::width).sum::<usize>() > room {
        // The most expendable, and the last of those, so a tie is broken in
        // favour of the hint nearer the front.
        let Some((index, _)) = kept
            .iter()
            .enumerate()
            .filter(|(_, hint)| hint.expendable > 0)
            .max_by_key(|(index, hint)| (hint.expendable, *index))
        else {
            // Only essentials left. They are shown even if they overflow: a
            // terminal too narrow for `q quit` is one where truncation is the
            // least of anybody's problems.
            break;
        };

        kept.remove(index);
        room = room.max(1);
    }

    kept
}

/// The status line: what can be pressed, and what needs attention.
///
/// Takes the width because it decides what fits. See [`Hint`].
pub fn status(app: &App, width: u16) -> Paragraph<'static> {
    let mut spans = Vec::new();

    // The question comes before everything else and in a colour that stops the
    // eye: the next `q` throws work away, and nothing else on this line
    // matters until it is answered.
    if app.wants_quit_confirmation() {
        spans.push(Span::styled(
            "unsaved changes — q again to discard, ctrl-s to save   ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ));
    }
    // A notice replaces the hints rather than crowding them: it reports what
    // an export just wrote, which is the only thing worth reading afterwards.
    // A notice replaces the hints rather than crowding them — but only while
    // it is still worth reading. Nothing used to clear one, so a single save
    // removed every key hint for the rest of the session.
    else if let Some(text) = app.notice.as_ref().and_then(super::app::Notice::text) {
        let colour = if app
            .notice
            .as_ref()
            .is_some_and(super::app::Notice::is_warning)
        {
            Color::LightRed
        } else {
            Color::LightGreen
        };
        spans.push(Span::styled(text.to_owned(), Style::default().fg(colour)));
        spans.push(Span::raw("   "));
    } else {
        // The markers are measured first: they are not hints and are never
        // dropped, so whatever they take is not room the hints have.
        let markers = usize::from(app.is_stale()) * "● on disk  ".len()
            + usize::from(app.is_dirty()) * "● unsaved  ".len()
            // Neither the model nor the agent's activity is a hint, and
            // neither is ever dropped, so whatever they take is not room
            // the hints have.
            + current_model(app).map_or(0, |model| model.chars().count() + 3)
            + agent_activity(app).map_or(0, |activity| activity.chars().count() + 3);

        let room = usize::from(width).saturating_sub(markers);

        for hint in fit(&hints(app), room) {
            spans.extend(hint.spans());
        }
    }

    // Only under `-v`: the number matters when a preview feels slow, and is
    // noise the rest of the time.
    if app.verbose > 0
        && let Some(elapsed) = app.last_render()
    {
        spans.push(Span::styled(
            format!("render {}ms   ", elapsed.as_millis()),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Somebody else's write, waiting to be taken. Distinct from unsaved and
    // worth more: unsaved means the file is behind the screen, stale means the
    // screen is behind the file, and only one of the two is fixed by saving.
    if app.is_stale() {
        spans.push(Span::styled(
            "● on disk  ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }

    // Outside the branches above, because a notice or a quit warning must not
    // be able to hide the fact that there is unsaved work.
    if app.is_dirty() {
        spans.push(Span::styled(
            "● unsaved  ",
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Shown whether or not the agent is doing anything — the `M` picker is
    // not the only place worth knowing which model is answering, and it is
    // the one thing this line said nothing about before.
    if let Some(model) = current_model(app) {
        spans.push(Span::styled(
            format!("   {model}"),
            Style::default().fg(Color::LightBlue),
        ));
    }

    if let Some(activity) = agent_activity(app) {
        spans.push(Span::styled(
            format!("   {activity}"),
            Style::default().fg(Color::LightCyan),
        ));
    }

    Paragraph::new(Line::from(spans))
}

/// The model currently in use, if the agent has said which one that is.
///
/// `None` until [`crate::acp::AgentUpdate::Models`] arrives — not an error,
/// only that this build has not been told yet, or the agent offers no such
/// choice at all — and `None` again if it arrived but marked no model
/// current, which the protocol allows.
fn current_model(app: &App) -> Option<&str> {
    app.models()
        .iter()
        .find(|model| model.current)
        .map(|model| model.name.as_str())
}

/// Which keys are worth naming, in the order they are read.
///
/// Follows the state, for room and for honesty: the keys that move between
/// tabs are not offered while a modal owns every one of them, and the prompt's
/// own two keys are not offered while the palette has the keyboard.
fn hints(app: &App) -> Vec<Hint> {
    // A modal owns every key, so listing the workspace's would be worse than
    // listing none. Its own keys are along the bottom of the dialogue, which
    // is where somebody looking at a dialogue is looking.
    if app.modal().is_some() {
        return vec![Hint::essential("esc", "close")];
    }

    if app.is_editing() {
        // The workspace's own keys all mean something else while a pane is
        // being edited.
        let waiting = app.transcript.is_busy();

        let mut hints = vec![
            Hint::essential("esc", "leave"),
            Hint::optional("tab", "pane", 2),
            // The same thing `a` does from the pane, without leaving.
            Hint::essential("alt+a", "send"),
        ];

        if waiting {
            hints.push(Hint::essential("ctrl-c", "stop"));
        }

        hints.push(Hint::optional("ctrl-s", "save", 1));
        return hints;
    }

    let mut hints = vec![
        Hint::optional("tab", "pane", 2),
        Hint::essential("enter", "edit"),
    ];

    if app.focus == Focus::Prompt && app.left_view() == LeftView::Prompt {
        // Essential, and this is the whole point of the type. `a` is the only
        // way to reach the agent, and it was invisible for as long as it was
        // the first thing dropped to make the numbers work.
        hints.push(Hint::essential("a", "send"));
        hints.push(Hint::optional("e", "$EDITOR", 3));
    }

    if app.focus == Focus::Palette {
        hints.push(Hint::optional("p", "picker", 8));
    }

    // The ordering of what is given up first, most eager last in this list.
    // There are far more keys than columns now, so this is where the line
    // decides what a narrow terminal is for: the toggles that have a visible
    // button on the toolbar go before the ones that do not.
    hints.push(Hint::optional("←→", "tab", 9));
    hints.push(Hint::optional("m", app.mode().other().title(), 5));
    hints.push(Hint::optional(
        "s",
        match app.view() {
            View::Preview => "source",
            View::Source => "preview",
        },
        2,
    ));
    hints.push(Hint::optional(
        "t",
        match app.left_view() {
            LeftView::Prompt => "transcript",
            LeftView::Transcript => "prompt",
        },
        6,
    ));
    hints.push(Hint::optional("x", app.mode().title(), 8));
    // Only once there is something to pick — a hint for an empty picker
    // would be one that lies.
    if !app.models().is_empty() {
        hints.push(Hint::optional("M", "model", 8));
    }
    hints.push(Hint::optional("r", "render", 4));
    hints.push(Hint::optional("ctrl-s", "save", 1));
    hints.push(Hint::optional("R", "reload", 7));
    hints.push(Hint::essential("q", "quit"));

    hints
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;

    use super::*;
    use crate::Project;
    use crate::fixtures;
    use crate::tui::app::Notice;

    /// Everything a widget drew, as one string.
    fn drawn(widget: impl Widget, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        widget.render(area, &mut buffer);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn an_agents_bold_text_is_drawn_without_its_asterisks() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.transcript.apply(crate::acp::AgentUpdate::Message(
            "This is **bold** text.".to_owned(),
        ));

        let output = drawn(Paragraph::new(transcript_lines(&app)), 60, 4);

        assert!(output.contains("This is bold text."), "{output}");
        assert!(
            !output.contains("**"),
            "the asterisks leaked through:\n{output}"
        );
    }

    #[test]
    fn a_bulleted_list_in_the_transcript_shows_a_real_bullet() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.transcript
            .apply(crate::acp::AgentUpdate::Message("- one\n- two".to_owned()));

        let output = drawn(Paragraph::new(transcript_lines(&app)), 60, 4);

        assert!(output.contains('\u{2022}'), "{output}");
        assert!(
            !output.contains("- one") && !output.contains("- two"),
            "the source dash leaked through:\n{output}"
        );
    }

    #[test]
    fn the_palette_pane_shows_each_colours_name_value_and_role() {
        let project = fixtures::project();
        let output = drawn(palette(&project.metadata().palette, true, None), 60, 4);

        assert!(output.contains("accent"), "{output}");
        assert!(output.contains("#f05032"), "{output}");
        assert!(output.contains("#18181b"), "{output}");
    }

    #[test]
    fn a_field_being_typed_into_is_drawn_instead_of_the_value_it_will_become() {
        // A hex colour does not parse until its last character, so drawing the
        // committed value would make every keystroke but the final one
        // invisible.
        let project = fixtures::project();
        let output = drawn(
            palette(
                &project.metadata().palette,
                true,
                Some(PaletteEdit {
                    row: 0,
                    field: PaletteField::Value,
                    text: "#0066",
                    valid: false,
                }),
            ),
            60,
            4,
        );

        assert!(output.contains("#0066"), "{output}");
        assert!(
            !output.contains("#f05032"),
            "the committed value is still there:\n{output}"
        );
    }

    #[test]
    fn a_project_with_no_prompt_says_so_rather_than_showing_an_empty_pane() {
        let mut project = fixtures::project();
        project.metadata_mut().prompt = None;
        let app = App::new(project, "blocks");

        assert!(drawn(prompt(&app), 60, 4).contains("No prompt"));
    }

    #[test]
    fn the_prompt_pane_draws_the_editor_while_editing() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_editor();
        app.set_draft("an edited prompt");

        let output = drawn(app.editor(), 60, 4);

        assert!(output.contains("an edited prompt"), "{output}");
    }

    #[test]
    fn a_prompt_wider_than_the_pane_wraps_rather_than_running_off_the_edge() {
        // A prompt is prose in a column barely thirty cells wide, so a text
        // area that scrolled horizontally instead would show one line of it.
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_editor();
        app.set_draft("wrapping is what makes this pane readable at all");

        let output = drawn(app.editor(), 20, 5);

        assert!(output.contains("wrapping"), "{output}");
        assert!(output.contains("readable"), "{output}");
    }

    #[test]
    fn the_editing_hints_name_the_way_out() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_editor();

        let output = drawn(status(&app, 120), 120, 1);

        assert!(output.contains("esc"), "{output}");
        assert!(output.contains("leave"), "{output}");
    }

    #[test]
    fn the_prompt_pane_advertises_the_two_keys_only_it_has() {
        // Nothing else in the workspace would ever reveal them.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;

        let output = drawn(status(&app, 120), 120, 1);

        assert!(output.contains("edit"), "{output}");
        assert!(output.contains("$EDITOR"), "{output}");
    }

    /// Every hint on the line, as one string.
    fn footer(app: &App, width: u16) -> String {
        drawn(status(app, width), width, 1)
    }

    #[test]
    fn the_model_hint_only_appears_once_the_agent_has_offered_some() {
        // A hint for a picker with nothing to pick would be one that lies.
        // Wide enough that nothing else on the line is dropped for room —
        // this is about the hint's own condition, not the line's truncation.
        let mut app = App::new(fixtures::project(), "blocks");
        assert!(!footer(&app, 200).contains('M'), "{}", footer(&app, 200));

        app.set_models(vec![crate::acp::ModelChoice {
            id: "opencode/grok-code".to_owned(),
            name: "Grok Code".to_owned(),
            current: true,
        }]);

        assert!(footer(&app, 200).contains('M'), "{}", footer(&app, 200));
    }

    #[test]
    fn the_current_model_is_shown_in_the_status_line() {
        let mut app = App::new(fixtures::project(), "blocks");
        assert!(
            !footer(&app, 200).contains("Grok Code"),
            "{}",
            footer(&app, 200)
        );

        app.set_models(vec![
            crate::acp::ModelChoice {
                id: "anthropic/claude-opus-4-1".to_owned(),
                name: "Claude Opus 4.1".to_owned(),
                current: false,
            },
            crate::acp::ModelChoice {
                id: "opencode/grok-code".to_owned(),
                name: "Grok Code".to_owned(),
                current: true,
            },
        ]);

        let output = footer(&app, 200);
        assert!(output.contains("Grok Code"), "{output}");
        assert!(
            !output.contains("Claude Opus 4.1"),
            "only the model marked current should show: {output}"
        );
    }

    #[test]
    fn the_essential_hints_survive_eighty_columns() {
        // The test this replaces asserted that *everything* fitted, and the
        // way to satisfy it was to stop naming `a` — the only key that reaches
        // the agent. What matters is not that every hint fits; it is that the
        // ones you cannot work without are never the ones dropped.
        for focus in Focus::ALL {
            let mut app = App::new(fixtures::project(), "blocks");
            app.focus = focus;
            app.agent = AgentStatus::Ready;
            app.set_draft("unsaved");

            let line = footer(&app, 80);

            assert!(line.contains("quit"), "{focus:?}: no way out\n{line}");
            assert!(
                line.contains("unsaved"),
                "{focus:?}: unsaved work went unreported\n{line}"
            );
            if focus == Focus::Prompt {
                assert!(
                    line.contains(" send"),
                    "{focus:?}: the agent is unreachable\n{line}"
                );
                assert!(line.contains("edit"), "{focus:?}\n{line}");
            }

            // And nothing was cut in half on its way off the end.
            assert!(
                !line.trim_end().ends_with(' ') || line.len() <= 80,
                "{focus:?}: the line was truncated mid-hint\n{line}"
            );
        }
    }

    #[test]
    fn the_prompt_pane_always_offers_a_way_to_reach_the_agent() {
        // Whatever the agent is doing, and whatever the terminal's width. The
        // reported bug was that `a` appeared nowhere at all: not in the line,
        // and in the title only once the agent was ready with an empty
        // transcript.
        for agent in [
            AgentStatus::Connecting,
            AgentStatus::Ready,
            AgentStatus::Absent {
                reason: "none".to_owned(),
            },
        ] {
            let mut app = App::new(fixtures::project(), "blocks");
            app.focus = Focus::Prompt;
            app.agent = agent.clone();

            let line = footer(&app, 80);
            assert!(line.contains(" send"), "{agent:?}\n{line}");
        }
    }

    #[test]
    fn every_hint_fits_eighty_columns_now_that_the_backend_has_moved() {
        // Nothing has to be given up at the width everybody has. Freeing the
        // fifteen columns the backend was taking paid for `a` outright.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;

        let line = footer(&app, 80);
        for expected in ["tab", "edit", "send", "$EDITOR", "source", "save", "quit"] {
            assert!(line.contains(expected), "{expected} missing from\n{line}");
        }
    }

    #[test]
    fn a_narrow_terminal_gives_up_the_least_important_hints_first() {
        // The point of measuring at all: `$EDITOR` and `reload` go, `send`
        // and `quit` stay. Nothing is lost permanently — it comes back with
        // the room.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;

        let narrow = footer(&app, 46);

        assert!(
            narrow.contains("send"),
            "the agent went unreachable\n{narrow}"
        );
        assert!(narrow.contains("quit"), "no way out\n{narrow}");
        assert!(
            !narrow.contains("$EDITOR"),
            "the least important hint outlived the most important\n{narrow}"
        );

        let wide = footer(&app, 140);
        assert!(wide.contains("$EDITOR"), "it did not come back\n{wide}");
    }

    #[test]
    fn a_terminal_too_narrow_for_anything_still_offers_a_way_out() {
        // Degrades rather than disappearing. Below this width the essentials
        // overflow, which is the least of anybody's problems.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;

        let line = footer(&app, 30);
        assert!(line.contains("quit") || line.contains("ask"), "{line}");
    }

    #[test]
    fn a_notice_stops_hiding_the_key_hints_once_it_is_old() {
        // Nothing cleared a notice, and the line shows one *instead of* the
        // hints — so a single save removed every key for the rest of the
        // session, including the one that reaches the agent.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;

        app.notice = Some(Notice::info("wrote logo.svg"));
        let fresh = drawn(status(&app, 100), 100, 1);
        assert!(fresh.contains("wrote logo.svg"), "{fresh}");
        assert!(!fresh.contains("quit"), "{fresh}");

        // Aged out. No timer and no keypress: the status line simply stops
        // asking for it, and the 250ms tick redraws.
        app.notice = Some(Notice::aged("wrote logo.svg"));
        let stale = drawn(status(&app, 100), 100, 1);
        assert!(!stale.contains("wrote logo.svg"), "{stale}");
        assert!(
            stale.contains("send"),
            "the hints did not come back\n{stale}"
        );
        assert!(stale.contains("quit"), "{stale}");
    }

    #[test]
    fn a_failure_is_readable_for_longer_than_a_confirmation() {
        // "wrote logo.svg" is a confirmation; "save failed: …" is something
        // to act on, and four seconds is not long enough to act.
        assert!(Notice::warning("x").lifetime() > Notice::info("x").lifetime());
    }

    #[test]
    fn the_status_line_shows_what_the_agent_is_doing() {
        // Sending used to have no visible effect until the transcript filled
        // in seconds later and off to the left.
        let mut app = App::new(fixtures::project(), "blocks");
        app.agent = AgentStatus::Busy;
        app.transcript.push_user("a mark".to_owned());
        app.transcript.apply(crate::acp::AgentUpdate::ToolStarted {
            id: "1".to_owned(),
            title: "render_svg".to_owned(),
        });

        let line = drawn(status(&app, 120), 120, 1);
        assert!(line.contains("render_svg"), "{line}");
    }

    #[test]
    fn a_connecting_agent_does_not_look_like_a_ready_one() {
        // They rendered identically, and connecting is the state in which a
        // prompt waits rather than runs.
        let mut app = App::new(fixtures::project(), "blocks");

        app.agent = AgentStatus::Connecting;
        let connecting = format!("{}|{}", agent_title(&app), drawn(status(&app, 120), 120, 1));

        app.agent = AgentStatus::Ready;
        let ready = format!("{}|{}", agent_title(&app), drawn(status(&app, 120), 120, 1));

        assert_ne!(connecting, ready);
        assert!(connecting.contains("starting"), "{connecting}");
    }

    #[test]
    fn the_pane_never_names_a_key_that_cannot_work_from_where_it_says_it() {
        // The title read "a to ask the agent" *while editing*, where `a` types
        // a letter. That is what sent a whole message into the project's
        // prompt with nothing reaching the agent.
        let mut app = App::new(fixtures::project(), "blocks");
        app.agent = AgentStatus::Ready;
        app.engage_editor();

        let title = agent_title(&app);
        assert!(
            !title.contains("  a to ask"),
            "the title names bare `a` while the editor would swallow it: {title}"
        );
        assert!(title.contains("alt+a"), "{title}");
    }

    #[test]
    fn the_status_line_names_the_chord_while_the_editor_has_the_keyboard() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_editor();
        assert!(drawn(status(&app, 80), 80, 1).contains("alt+a"));

        app.engage_editor();
        assert!(drawn(status(&app, 80), 80, 1).contains("alt+a"));
    }

    #[test]
    fn the_pane_title_says_how_to_reach_the_agent() {
        // Where `a` is discovered, since the status line has no room. Only
        // while there is an agent: advertising a key that answers "no agent is
        // connected" is worse than saying nothing.
        let mut app = App::new(fixtures::project(), "blocks");
        assert_eq!(agent_title(&app), "");

        app.agent = AgentStatus::Ready;
        assert!(agent_title(&app).contains('a'), "{}", agent_title(&app));
    }

    #[test]
    fn the_status_line_distinguishes_unsaved_work_from_a_change_on_disk() {
        // Two different problems with two different answers: one is fixed by
        // saving, the other is made worse by it.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        let mut app = App::new(Project::open(&path).unwrap(), "blocks");

        app.set_draft("mine");
        let unsaved = drawn(status(&app, 120), 120, 1);
        assert!(unsaved.contains("unsaved"), "{unsaved}");
        assert!(!unsaved.contains("on disk"), "{unsaved}");

        std::fs::write(&path, fixtures::PROJECT.replace("#f05032", "#0066ff")).unwrap();
        app.poll_file();

        let stale = drawn(status(&app, 120), 120, 1);
        assert!(stale.contains("on disk"), "{stale}");
    }

    #[test]
    fn the_status_line_says_when_there_is_unsaved_work() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.set_draft("changed");

        assert!(drawn(status(&app, 120), 120, 1).contains("unsaved"));
    }

    #[test]
    fn the_status_line_asks_before_unsaved_work_is_discarded() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.set_draft("changed");
        app.request_quit();

        let output = drawn(status(&app, 120), 120, 1);
        assert!(output.contains("q again to discard"), "{output}");
    }

    #[test]
    fn a_notice_replaces_the_key_hints_so_it_is_actually_read() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.notice = Some(Notice::info("wrote dist/favicon-32.png"));
        let output = drawn(status(&app, 90), 90, 1);

        assert!(output.contains("wrote dist/favicon-32.png"), "{output}");
        // And the hints stand aside for it entirely.
        assert!(!output.contains("quit"), "{output}");
    }

    #[test]
    fn the_status_line_leaves_the_backend_to_the_preview_pane() {
        // It cost fifteen columns of every row on every pane to say something
        // about one pane — and those were the columns the prompt pane needed
        // to name the key that reaches the agent.
        let app = App::new(fixtures::project(), "kitty");
        assert!(!drawn(status(&app, 120), 120, 1).contains("kitty"));
    }
}
