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

use super::app::{App, Focus, Preview};
use super::panes;

/// Draw a frame.
pub fn draw(frame: &mut Frame<'_>, app: &App, backend: &mut Backend) {
    let [body, status] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .areas(frame.area());

    // The left column is sized in characters, not proportions: its contents
    // are names and hex values of known width, and the preview should take
    // every column they do not need.
    let [left, right] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(38), Constraint::Min(20)])
        .areas(body);

    draw_left(frame, app, left);
    draw_preview(frame, app, backend, right);

    frame.render_widget(panes::status(app), status);
}

/// Draw the description column.
fn draw_left(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let metadata = app.project.metadata();

    // The prompt is prose of unknown length and the lists are not, so the
    // lists get exactly the rows they need and the prompt gets the rest.
    let rows = |count: usize| Constraint::Length(u16::try_from(count).unwrap_or(u16::MAX) + 2);
    let [prompt, palette, variants, renders] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            rows(metadata.palette.len()),
            rows(metadata.variants.len()),
            rows(metadata.renders.len()),
        ])
        .areas(area);

    let framed = |focus: Focus| panes::frame(focus, app.focus == focus);

    frame.render_widget(panes::prompt(app).block(framed(Focus::Prompt)), prompt);
    frame.render_widget(
        panes::palette(
            &metadata.palette,
            app.selection(Focus::Palette),
            app.focus == Focus::Palette,
        )
        .block(framed(Focus::Palette)),
        palette,
    );
    frame.render_widget(
        panes::variants(
            &metadata.variants,
            metadata.primary.as_deref(),
            app.selection(Focus::Variants),
            app.focus == Focus::Variants,
        )
        .block(framed(Focus::Variants)),
        variants,
    );
    frame.render_widget(
        panes::renders(
            &metadata.renders,
            app.selection(Focus::Renders),
            app.focus == Focus::Renders,
        )
        .block(framed(Focus::Renders)),
        renders,
    );
}

/// Draw the preview column.
fn draw_preview(frame: &mut Frame<'_>, app: &App, backend: &mut Backend, area: Rect) {
    let caption = match app.preview() {
        Preview::Ready { caption, .. } => format!(" preview — {caption} "),
        _ => " preview ".to_owned(),
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
            backend.draw(image, app.preview_generation(), inner, frame);
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
    fn frame(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut backend = Backend::detect(PreviewBackend::Blocks);
        terminal
            .draw(|frame| draw(frame, app, &mut backend))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// The text of a frame.
    fn render(app: &App, width: u16, height: u16) -> String {
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
    fn a_frame_shows_every_pane_and_the_key_hints() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        let text = render(&app, 100, 30);

        for expected in [
            "prompt", "palette", "variants", "renders", "preview", "quit",
        ] {
            assert!(text.contains(expected), "{expected} missing from:\n{text}");
        }
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

        let drawn = painted(&frame(&app, 100, 30), PREVIEW);
        assert!(drawn > 100, "only {drawn} cells were painted");
    }

    #[test]
    fn a_failed_preview_shows_the_reason_instead_of_an_image() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.project.metadata_mut().variants.clear();
        app.invalidate_preview();
        app.refresh_preview();

        let rendered = frame(&app, 100, 30);
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
    fn drawing_into_a_tiny_terminal_does_not_panic() {
        // Terminals get dragged to absurd sizes, and a panic here leaves the
        // user in raw mode on the alternate screen.
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        for (width, height) in [(1, 1), (5, 3), (40, 2), (200, 60)] {
            render(&app, width, height);
        }
    }
}
