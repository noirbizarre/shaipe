//! The widgets each pane draws.
//!
//! One function per pane, each taking the state it needs and nothing more.
//! They build widgets and return them; where those widgets go is [`super::ui`]'s
//! decision, so a change to the layout does not touch a pane and a change to a
//! pane does not touch the layout.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};

use crate::project::{Palette, RenderSpec, Rgba, Variant};

use super::app::{AgentStatus, App, Focus};
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
pub fn prompt(app: &App, height: u16) -> Paragraph<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // The project's own prompt first, then the conversation about it. The
    // editor is drawn separately, on the pane's last rows, so it cannot be
    // pushed off the bottom by a transcript that keeps growing.
    match &app.project.metadata().prompt {
        Some(prompt) => lines.push(Line::styled(
            prompt.clone(),
            Style::default().fg(Color::Gray),
        )),
        None => lines.push(Line::styled(
            "No prompt recorded.",
            Style::default().fg(Color::DarkGray),
        )),
    }

    // Why the agent cannot be asked, said before it is reached for rather than
    // after a message has been typed and swallowed.
    if let AgentStatus::Absent { reason } = &app.agent {
        lines.push(Line::raw(""));
        for line in reason.lines() {
            lines.push(Line::styled(
                line.to_owned(),
                Style::default().fg(Color::Yellow),
            ));
        }
    }

    if !app.transcript.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(transcript(app));
    }

    // Anchored to the bottom, so a conversation scrolls the way every other
    // conversation does: the newest thing is the thing you can see.
    //
    // Counted in lines rather than in wrapped rows, so a long entry can still
    // push a little too far. Exact scrolling needs the wrap width and the
    // widget's own line breaking, and being approximately right at the bottom
    // beats being exactly right at the top.
    let overflow = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(height);

    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((overflow, 0))
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

/// What the agent is doing, for the right-hand end of the status line.
///
/// A spinner and a word, derived from an `Instant` at draw time so that no
/// animation state is stored anywhere — the same shape as [`App::spinner`].
/// Without this the only sign that a prompt had been sent was the transcript
/// filling in, which happens seconds later and off to the left.
fn agent_activity(app: &App) -> Option<String> {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    const PERIOD: u128 = 90;

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

    let step = app.agent_since().elapsed().as_millis() / PERIOD;
    Some(format!(
        "{} {doing}",
        FRAMES[(step as usize) % FRAMES.len()]
    ))
}

/// The conversation, one entry at a time.
fn transcript(app: &App) -> Vec<Line<'static>> {
    app.transcript
        .entries()
        .iter()
        .map(|entry| match entry {
            Entry::You(text) => Line::from(vec![
                Span::styled("you  ", Style::default().fg(Color::LightMagenta)),
                Span::raw(text.clone()),
            ]),
            Entry::Agent(text) => Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::raw(text.clone()),
            ]),
            // Dimmed and marked, because reasoning read as a statement is how
            // a person ends up believing the agent said something it did not.
            Entry::Thought(text) => Line::styled(
                format!("     {text}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
            Entry::Tool { title, status, .. } => Line::from(vec![
                Span::styled(
                    format!("  {} ", status.glyph()),
                    Style::default().fg(match status {
                        ToolStatus::Running => Color::Yellow,
                        ToolStatus::Completed => Color::Green,
                        ToolStatus::Failed => Color::Red,
                    }),
                ),
                Span::styled(title.clone(), Style::default().fg(Color::Cyan)),
            ]),
            Entry::Notice(text) => {
                Line::styled(format!("     {text}"), Style::default().fg(Color::Yellow))
            }
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
pub fn palette(palette: &Palette, focused: bool) -> List<'static> {
    let items: Vec<ListItem> = palette
        .colours()
        .iter()
        .map(|colour| {
            let mut spans = vec![
                swatch(colour.value),
                Span::raw(" "),
                Span::raw(colour.name.clone()),
                Span::raw("  "),
                Span::styled(
                    colour.value.to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
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

/// The variants pane.
pub fn variants(variants: &[Variant], primary: Option<&str>, focused: bool) -> List<'static> {
    let items: Vec<ListItem> = variants
        .iter()
        .map(|variant| {
            let mut spans = vec![Span::raw(variant.name.clone())];
            // Which variant the document's root draws is otherwise invisible,
            // and it is the one that shows up on GitHub.
            if primary == Some(variant.name.as_str()) {
                spans.push(Span::styled(" ●", Style::default().fg(Color::LightMagenta)));
            }
            spans.push(Span::styled(
                format!("  #{}", variant.element),
                Style::default().fg(Color::DarkGray),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    List::new(items).highlight_style(selected(focused))
}

/// The render specifications pane.
pub fn renders(specs: &[RenderSpec], focused: bool) -> List<'static> {
    let items: Vec<ListItem> = specs
        .iter()
        .map(|spec| {
            ListItem::new(Line::from(vec![
                Span::raw(spec.name.clone()),
                Span::styled(
                    format!("  {}x{}", spec.width, spec.height),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("  {}", spec.variant),
                    Style::default().fg(Color::Blue),
                ),
            ]))
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
            // The agent's activity is not a hint and is never dropped, so
            // whatever it takes is not room the hints have.
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

    if let Some(activity) = agent_activity(app) {
        spans.push(Span::styled(
            format!("   {activity}"),
            Style::default().fg(Color::LightCyan),
        ));
    }

    Paragraph::new(Line::from(spans))
}

/// Which keys are worth naming, in the order they are read.
///
/// Follows the focus, for room and for honesty: the prompt pane has no rows,
/// so `↑↓` never moved anything there, and re-rendering is not offered from it
/// because a prompt is metadata and cannot change what the preview shows.
fn hints(app: &App) -> Vec<Hint> {
    if app.is_editing() {
        // The workspace's own keys all mean something else while the editor
        // has the keyboard, and listing those would be worse than listing none.
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

    let mut hints = vec![Hint::optional("tab", "pane", 2)];

    if app.focus == Focus::Prompt {
        hints.push(Hint::essential("enter", "edit"));
        // Essential, and this is the whole point of the type. `a` is the only
        // way to reach the agent, and it was invisible for as long as it was
        // the first thing dropped to make the numbers work.
        hints.push(Hint::essential("a", "send"));
        hints.push(Hint::optional("e", "$EDITOR", 4));
    } else {
        hints.push(Hint::optional("↑↓", "select", 3));
        hints.push(Hint::optional("r", "render", 3));
    }

    hints.push(Hint::optional(
        "s",
        if app.shows_source() {
            "preview"
        } else {
            "source"
        },
        3,
    ));
    hints.push(Hint::optional("ctrl-s", "save", 1));
    hints.push(Hint::optional("R", "reload", 5));
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
    fn the_palette_pane_shows_each_colours_name_value_and_role() {
        let project = fixtures::project();
        let output = drawn(palette(&project.metadata().palette, true), 60, 4);

        assert!(output.contains("accent"), "{output}");
        assert!(output.contains("#f05032"), "{output}");
        assert!(output.contains("#18181b"), "{output}");
    }

    #[test]
    fn the_variants_pane_marks_the_primary_variant() {
        let project = fixtures::project();
        let metadata = project.metadata();
        let output = drawn(
            variants(&metadata.variants, metadata.primary.as_deref(), true),
            60,
            4,
        );

        assert!(output.contains("icon ●"), "{output}");
        assert!(output.contains("#mark-wide"), "{output}");
    }

    #[test]
    fn the_renders_pane_shows_each_specifications_size_and_variant() {
        let project = fixtures::project();
        let output = drawn(renders(&project.metadata().renders, true), 60, 4);

        assert!(output.contains("favicon-32"), "{output}");
        assert!(output.contains("128x32"), "{output}");
    }

    #[test]
    fn a_project_with_no_prompt_says_so_rather_than_showing_an_empty_pane() {
        let mut project = fixtures::project();
        project.metadata_mut().prompt = None;
        let app = App::new(project, "blocks");

        assert!(drawn(prompt(&app, 4), 60, 4).contains("No prompt"));
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
