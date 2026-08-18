//! Where the panes go.
//!
//! The preview draws through [`crate::preview::Preview`], which is a widget
//! like everything else here, so this module never touches an escape
//! sequence and never needs to know which protocol is in use.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Paragraph, Wrap};

use crate::preview::Preview as Backend;

use super::app::{App, EditorMode, Focus, Preview};
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
    app.set_preview_area(right, backend.cell_size(), backend.scale());
    draw_preview(frame, app, backend, right);

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
    let prompt_block = panes::frame_titled(
        Focus::Prompt,
        focused(Focus::Prompt),
        app.mode().title(),
        panes::agent_title(app),
    );
    let interior = prompt_block.inner(prompt);
    frame.render_widget(&prompt_block, prompt);

    if app.is_editing() && interior.height > 0 {
        // The editor gets the bottom of the pane and the conversation the
        // rest. The other way round, a transcript that keeps growing would
        // push the line being typed off the bottom, and a field nobody can see
        // is one nobody can type into.
        //
        // Editing the project's prompt is prose and wants room; composing a
        // message is one thing said once and wants a line or two. Neither is
        // allowed to take the whole pane while there is a conversation to read.
        let wanted = match app.mode() {
            EditorMode::Prompt => interior.height.saturating_sub(1).max(1),
            EditorMode::Ask => 3.min(interior.height),
        };
        let [above, editing] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(wanted)])
            .areas(interior);

        if above.height > 0 {
            frame.render_widget(panes::prompt(app, above.height), above);
        }
        frame.render_widget(app.editor(), editing);
    } else if interior.height > 0 {
        frame.render_widget(panes::prompt(app, interior.height), interior);
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

/// Draw the preview column.
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

    #[test]
    fn the_editor_stays_on_screen_however_much_is_above_it() {
        // The editor is given the bottom of the pane rather than flowing after
        // the content, because the content grows without bound: the project's
        // prompt, then why there is no agent, then every turn of a
        // conversation. A field that scrolls out of view is one nobody can
        // type into, and the pane would look inert rather than full.
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_ask();
        app.set_draft("STILLVISIBLE");

        for turn in 0..40 {
            app.transcript.push_user(format!("turn number {turn}"));
            app.transcript
                .apply(crate::acp::AgentUpdate::Message(format!("answer {turn}")));
        }

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("STILLVISIBLE"),
            "the editor was pushed off the pane by the transcript:\n{text}"
        );
    }

    #[test]
    fn the_transcript_shows_its_newest_entries_rather_than_its_oldest() {
        // A conversation scrolls the way every other conversation does.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;

        for turn in 0..30 {
            app.transcript.push_user(format!("question {turn}"));
        }

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("question 29"),
            "the newest entry is not on screen:\n{text}"
        );
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
