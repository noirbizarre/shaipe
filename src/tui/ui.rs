//! Where the panes go.
//!
//! The preview draws through [`crate::preview::Preview`], which is a widget
//! like everything else here, so this module never touches an escape
//! sequence and never needs to know which protocol is in use.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::preview::Preview as Backend;

use super::app::{App, Focus, Preview, View};
use super::panes;

/// Draw a frame.
pub fn draw(frame: &mut Frame<'_>, app: &mut App, backend: &mut Backend) {
    let [body, status] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .areas(frame.area());

    // Sized in characters, not proportions: the column's contents are names
    // and hex values of known width, and the preview should have every column
    // they do not need. Draggable, hence read from the app rather than fixed.
    app.resize_column(app.column_width, body.width);
    let [left, right] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(app.column_width),
            Constraint::Min(crate::tui::app::MIN_PREVIEW),
        ])
        .areas(body);

    draw_left(frame, app, left);
    // Told about the area either way, so the preview stays correctly sized
    // and switching back to it is instant rather than a re-render.
    app.set_preview_area(right, backend.cell_size(), backend.scale());
    match app.view() {
        View::Preview => draw_preview(frame, app, backend, right),
        View::Source => draw_scrolling(frame, app, right, View::Source),
        View::Log => draw_scrolling(frame, app, right, View::Log),
    }

    frame.render_widget(panes::status(app, status.width), status);
}

/// Rows an unfocused pane may occupy, borders included.
///
/// Small enough that focusing a pane visibly hands it the column, large
/// enough that a collapsed pane still shows something. A collapsed list
/// scrolls, so its selection stays visible.
const COLLAPSED: u16 = 5;

/// The height each pane should get.
///
/// The focused pane takes everything the others do not need; the others take
/// what their contents want, up to [`COLLAPSED`]. This is what makes the
/// prompt readable — as prose it is the one pane whose content has no natural
/// height, and before this it was whatever the three lists left over.
fn constraints(app: &App) -> [Constraint; Focus::COUNT] {
    std::array::from_fn(|index| {
        let focus = Focus::ALL[index];
        if focus == app.focus {
            Constraint::Min(5)
        } else {
            Constraint::Length(app.natural_height(focus).min(COLLAPSED))
        }
    })
}

/// Draw the description column.
fn draw_left(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let [prompt, palette, variants, renders] = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints(app))
        .areas(area);

    for (focus, pane) in [
        (Focus::Prompt, prompt),
        (Focus::Palette, palette),
        (Focus::Variants, variants),
        (Focus::Renders, renders),
    ] {
        app.set_area(focus, pane);
    }

    // A value, not a closure: the closure would borrow `app` immutably for the
    // rest of the function, and the list panes need it mutably for their
    // scroll state.
    let focus = app.focus;
    let focused = move |candidate: Focus| candidate == focus;

    // The frame is drawn separately and its interior filled in, rather than
    // the editor carrying a block of its own: a block would have to be set on
    // the text area, and that mutable borrow cannot coexist with the immutable
    // one the list panes take below.
    let agent = panes::agent_title(app);
    let prompt_block = panes::frame_titled(
        Focus::Prompt,
        focused(Focus::Prompt),
        Focus::Prompt.title(),
        &agent,
    );

    // The frame is drawn separately and its interior filled in, rather than
    // the editor carrying a block of its own: a block would have to be set on
    // the text area, and that mutable borrow cannot coexist with the immutable
    // one the list panes take below.
    //
    // The editor takes the whole interior. It briefly shared it with the
    // agent's transcript, which was a mistake twice over — it left the editor
    // one row, and the transcript never had the height to be read in anyway.
    // The transcript lives in the right-hand column now.
    let interior = prompt_block.inner(prompt);
    frame.render_widget(&prompt_block, prompt);

    if interior.height > 0 {
        if app.is_editing() {
            frame.render_widget(app.editor(), interior);
        } else {
            frame.render_widget(panes::prompt(app), interior);
        }
    }

    // Built before the mutable borrow of `app` that the list state needs.
    let metadata = app.project.metadata();
    let lists = [
        (
            Focus::Palette,
            palette,
            panes::palette(&metadata.palette, focused(Focus::Palette)),
        ),
        (
            Focus::Variants,
            variants,
            panes::variants(
                &metadata.variants,
                metadata.primary.as_deref(),
                focused(Focus::Variants),
            ),
        ),
        (
            Focus::Renders,
            renders,
            panes::renders(&metadata.renders, focused(Focus::Renders)),
        ),
    ];

    for (focus, pane, list) in lists {
        let list = list.block(panes::frame(focus, focused(focus)));
        frame.render_stateful_widget(list, pane, app.list_state(focus));
    }
}

/// Draw one of the scrolling views in place of the picture.
///
/// The source and the log differ only in what they contain and how they wrap,
/// so they share a frame, a scroll offset and a set of keys.
fn draw_scrolling(frame: &mut Frame<'_>, app: &App, area: Rect, view: View) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            format!(" {} ", view.title()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let paragraph = match view {
        // Not wrapped: an SVG has meaningful line structure, and rewrapping a
        // path's coordinates across a narrow column makes it unreadable. It
        // runs off the edge instead, which is the lesser harm.
        View::Source => Paragraph::new(app.source_text()).style(Style::default().fg(Color::Gray)),
        // Wrapped, because it is prose.
        View::Log => Paragraph::new(panes::log(app)).wrap(Wrap { trim: false }),
        View::Preview => return,
    };

    // Anchored to the bottom, measured in **wrapped rows**. Counting entries
    // instead is the bug that kept the transcript off screen for a whole
    // session: one 446-character prompt is a single line and fifteen rows, so
    // the offset came out fourteen rows short and usually zero.
    let rows = u16::try_from(paragraph.line_count(inner.width)).unwrap_or(u16::MAX);
    let bottom = rows.saturating_sub(inner.height);
    let scroll = match view {
        // The newest entry is the one worth seeing, until somebody scrolls.
        View::Log => bottom.saturating_sub(app.view_scroll()),
        _ => app.view_scroll(),
    };

    frame.render_widget(paragraph.scroll((scroll, 0)), inner);
}

/// Draw the preview column./// Draw the preview column.
fn draw_preview(frame: &mut Frame<'_>, app: &mut App, backend: &mut Backend, area: Rect) {
    // The backend belongs here rather than in the status line, which was
    // spending fifteen columns of every row on every pane to say something
    // about this one — and those were the columns the prompt pane needed to
    // name the key that reaches the agent.
    //
    // It is read when a preview looks wrong, which is when the eye is already
    // on this pane.
    let caption = match app.preview() {
        Preview::Ready { caption, .. } => {
            format!(" preview ({}) — {caption} ", app.backend)
        }
        _ => format!(" preview ({}) ", app.backend),
    };
    // In the title rather than over the image: a render can take a noticeable
    // moment, and replacing the previous preview with a spinner would be a
    // downgrade. The old image is more useful than an empty pane.
    let caption = match app.spinner() {
        Some(frame) => format!("{caption}{frame} rendering "),
        None => caption,
    };

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(ratatui::text::Span::styled(
            caption,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match app.preview() {
        Preview::Pending => frame.render_widget(Paragraph::new("rendering…"), inner),
        Preview::Failed(reason) => frame.render_widget(
            Paragraph::new(reason.clone())
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::LightRed)),
            inner,
        ),
        Preview::Ready { image, .. } => {
            // Withheld for exactly one frame after a new image arrives, so the
            // spinner reaches the screen before the write that blocks on it.
            if app.is_holding_image() {
                frame.render_widget(
                    Paragraph::new("sending to the terminal…")
                        .style(Style::default().fg(Color::DarkGray)),
                    inner,
                );
            } else {
                backend.draw(image, app.preview_generation(), inner, frame);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::fixtures;
    use crate::preview::Backend as PreviewBackend;

    /// Draw one frame into an in-memory terminal.
    ///
    /// Half-blocks are forced: the other protocols emit escape sequences that
    /// a `TestBackend` records as opaque cell contents, which would make these
    /// assertions depend on whichever terminal happened to run the tests.
    fn frame(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut backend = Backend::detect(PreviewBackend::Blocks);
        terminal
            .draw(|frame| draw(frame, app, &mut backend))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// The text of a frame.
    fn render(app: &mut App, width: u16, height: u16) -> String {
        let buffer = frame(app, width, height);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// How many cells in `area` have been given a background colour.
    ///
    /// The evidence that an image was drawn, for two reasons. Not the `▀`
    /// character, because a region of uniform colour collapses to a coloured
    /// space — which is exactly what flat artwork produces. And not the
    /// foreground either, because styled *text* sets that, so counting it
    /// would make an error message look like a picture.
    fn painted(buffer: &ratatui::buffer::Buffer, area: Rect) -> usize {
        (area.y..area.y + area.height)
            .flat_map(|y| (area.x..area.x + area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buffer[(x, y)].bg != Color::Reset)
            .count()
    }

    #[test]
    fn an_engaged_editor_takes_over_the_prompt_pane_of_a_whole_frame() {
        // Through `draw` rather than the widget alone: this is the path where
        // the editor's mutable borrow of the state has to coexist with the
        // list panes' immutable borrow of the project.
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_editor();
        app.set_draft("an engaged editor");

        let text = render(&mut app, 100, 30);

        assert!(text.contains("an engaged editor"), "{text}");
        // The rest of the workspace is still drawn around it.
        assert!(text.contains("variants"), "{text}");
    }

    #[test]
    fn a_frame_shows_every_pane_and_the_key_hints() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        let text = render(&mut app, 100, 30);

        for expected in [
            "prompt", "palette", "variants", "renders", "preview", "quit",
        ] {
            assert!(text.contains(expected), "{expected} missing from:\n{text}");
        }
    }

    /// A prompt like a real one: several paragraphs, hundreds of characters.
    ///
    /// The fixture's is `A square and a bar.` — nineteen characters, one
    /// wrapped row — which is why counting lines where rows were meant looked
    /// correct in every test while being fourteen rows out in the workspace.
    const LONG_PROMPT: &str = "\
A mark for Shaipe, an LLM-native SVG asset workspace. The name plays on \
\"shape\" and \"AI\".\n\nA shape that is partly drawn and partly inferred: an \
outlined square whose lower-right quadrant has been resolved into solid \
colour, suggesting a form being completed rather than one already \
finished.\n\nGeometric, flat, monoline, no gradients. The icon carries no \
text, so it reads at 16 pixels.";

    #[test]
    fn the_editor_gets_the_whole_pane_while_editing() {
        // It briefly shared the pane with the transcript, which left it one
        // row and the transcript none. The transcript lives elsewhere now.
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_editor();
        app.set_draft("EDITING HERE");

        let text = render(&mut app, 100, 30);
        assert!(text.contains("EDITING HERE"), "{text}");
    }

    #[test]
    fn a_long_prompt_is_read_from_the_top() {
        // Not bottom-anchored: a prompt is a thing you wrote, not a
        // conversation with a newest end.
        let mut app = App::new(fixtures::project(), "blocks");
        app.project.metadata_mut().prompt = Some(LONG_PROMPT.to_owned());
        app.focus = Focus::Prompt;

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("A mark for Shaipe"),
            "the prompt did not start at the beginning:\n{text}"
        );
    }

    #[test]
    fn the_log_shows_its_newest_entries_even_when_they_wrap() {
        // The regression test for the whole business. The old one passed only
        // because the fixture's prompt was one row long, so counting entries
        // happened to equal counting rows. Here every entry wraps to several
        // rows, and an offset measured in entries lands nowhere near the end.
        let mut app = App::new(fixtures::project(), "blocks");
        app.project.metadata_mut().prompt = Some(LONG_PROMPT.to_owned());

        for turn in 0..12 {
            app.transcript
                .push_user(format!("{LONG_PROMPT} — asked{turn}"));
            app.transcript
                .apply(crate::acp::AgentUpdate::Message(format!(
                    "{LONG_PROMPT} — answered{turn}"
                )));
        }
        app.cycle_view();
        while app.view() != crate::tui::app::View::Log {
            app.cycle_view();
        }

        let text = render(&mut app, 100, 30);
        // A single unbroken word, because the screen is rows and a phrase
        // would be split across two of them by the wrap this test is about.
        assert!(
            text.contains("answered11"),
            "the newest entry is not on screen:\n{text}"
        );
        assert!(
            !text.contains("answered0 "),
            "it is showing the oldest instead:\n{text}"
        );
    }

    #[test]
    fn the_log_says_what_to_do_when_it_is_empty() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.cycle_view();
        while app.view() != crate::tui::app::View::Log {
            app.cycle_view();
        }

        let text = render(&mut app, 100, 30);
        assert!(text.contains("Nothing yet"), "{text}");
    }

    #[test]
    fn the_prompt_pane_says_why_there_is_no_agent() {
        // The diagnostic belongs where the person who cannot ask for anything
        // is looking. Said before a message is typed and swallowed, because
        // that reads like a bug rather than like a missing dependency.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;
        app.agent = crate::tui::app::AgentStatus::Absent {
            reason: "cannot find `no-such-agent`".to_owned(),
        };

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("no-such-agent"),
            "the pane did not say why the agent is missing:\n{text}"
        );
    }

    /// The preview pane's interior, for a 100x30 frame.
    const PREVIEW: Rect = Rect {
        x: 40,
        y: 1,
        width: 58,
        height: 27,
    };

    #[test]
    fn a_ready_preview_draws_pixels_in_the_right_hand_column() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();

        let drawn = painted(&frame(&mut app, 100, 30), PREVIEW);
        assert!(drawn > 100, "only {drawn} cells were painted");
    }

    #[test]
    fn a_failed_preview_shows_the_reason_instead_of_an_image() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.project.metadata_mut().variants.clear();
        app.invalidate_preview();
        app.refresh_preview();

        let rendered = frame(&mut app, 100, 30);
        let text = (0..30)
            .map(|y| {
                (0..100)
                    .map(|x| rendered[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("no variants"), "{text}");
        assert_eq!(
            painted(&rendered, PREVIEW),
            0,
            "an error message must not have an image drawn over it"
        );
    }

    #[test]
    fn the_focused_pane_gets_the_column_and_the_others_collapse() {
        // The prompt is prose with no natural height. Before this it got
        // whatever the three lists left over, which for a real project was a
        // handful of rows for several hundred characters.
        let mut app = App::new(fixtures::project(), "blocks");

        app.focus = Focus::Prompt;
        frame(&mut app, 100, 30);
        let prompt_focused = app.area(Focus::Prompt).height;

        app.focus = Focus::Renders;
        frame(&mut app, 100, 30);
        let prompt_collapsed = app.area(Focus::Prompt).height;
        let renders_focused = app.area(Focus::Renders).height;

        assert!(
            prompt_focused > prompt_collapsed * 2,
            "focusing the prompt should visibly hand it the column: \
             {prompt_focused} vs {prompt_collapsed}"
        );
        assert!(renders_focused > prompt_collapsed);
    }

    #[test]
    fn a_collapsed_pane_still_shows_its_title() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;
        let text = render(&mut app, 100, 30);

        // Collapsed, but not gone: the point is to see the whole project at
        // once and still be able to read the focused pane.
        for title in ["palette", "variants", "renders"] {
            assert!(text.contains(title), "{title} missing from:\n{text}");
        }
    }

    #[test]
    fn a_pane_smaller_than_its_contents_scrolls_to_keep_the_selection_visible() {
        // Collapsing a pane is only acceptable because it scrolls; without
        // this the selection could sit off-screen with no way to see it.
        let mut app = App::new(fixtures::project(), "blocks");
        for index in 0..10 {
            app.project
                .metadata_mut()
                .renders
                .push(crate::project::RenderSpec::square(
                    format!("spec-{index}"),
                    "icon",
                    16,
                ));
        }

        app.focus = Focus::Renders;
        while app.selection(Focus::Renders) + 1 < app.project.metadata().renders.len() {
            app.select_next();
        }

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("spec-9"),
            "the selected entry should have scrolled into view:\n{text}"
        );
    }

    #[test]
    fn a_running_render_puts_a_spinner_on_the_screen() {
        // The reported symptom was an empty preview box with no sign that
        // anything was happening. `App::spinner` being `Some` is not enough —
        // it has to reach the frame.
        let mut app = App::new(fixtures::project(), "blocks");
        assert!(app.begin_render(), "a render should have started");
        assert!(app.is_rendering());

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("rendering"),
            "no spinner reached the screen:\n{text}"
        );
    }

    #[test]
    fn the_frame_before_a_new_image_says_so_instead_of_drawing_it() {
        // Writing an image is what blocks, and it happens inside the draw
        // call, so nothing can animate during it. One cheap frame goes out
        // first to say the workspace is busy rather than wedged.
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();

        app.hold_image();
        let held = frame(&mut app, 100, 30);
        let text: String = (0..30)
            .map(|y| (0..100).map(|x| held[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("sending to the terminal"), "{text}");
        assert_eq!(
            painted(&held, PREVIEW),
            0,
            "the image must not be drawn on the frame that announces it"
        );

        app.release_image();
        assert!(painted(&frame(&mut app, 100, 30), PREVIEW) > 100);
    }

    #[test]
    fn drawing_into_a_tiny_terminal_does_not_panic() {
        // Terminals get dragged to absurd sizes, and a panic here leaves the
        // user in raw mode on the alternate screen.
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        for (width, height) in [(1, 1), (5, 3), (40, 2), (200, 60)] {
            render(&mut app, width, height);
        }
    }
}
