//! Where the panes go.
//!
//! [`draw`] returns the area the preview occupies, because a backend that
//! writes escape sequences straight to the terminal needs to know where to put
//! them and cannot find out from inside ratatui's buffer.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Paragraph, Wrap};

use crate::preview::blocks::HalfBlocks;

use super::app::{App, Focus, Preview};
use super::panes;

/// Draw a frame, returning the area a graphics backend should draw into.
///
/// `None` when there is nothing to draw there, so a backend is never asked to
/// place an image over an error message.
pub fn draw(frame: &mut Frame<'_>, app: &App) -> Option<Rect> {
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
    let preview = draw_preview(frame, app, right);

    frame.render_widget(panes::status(app), status);
    preview
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

/// Draw the preview column, returning the area inside its border.
fn draw_preview(frame: &mut Frame<'_>, app: &App, area: Rect) -> Option<Rect> {
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
        Preview::Pending => {
            frame.render_widget(Paragraph::new("rendering…"), inner);
            None
        }
        Preview::Failed(reason) => {
            frame.render_widget(
                Paragraph::new(reason.clone())
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(Color::LightRed)),
                inner,
            );
            None
        }
        Preview::Ready { image, .. } => {
            // Always drawn with half-blocks, even when a graphics backend is
            // about to draw over the top. A terminal that ignores the escape
            // sequence then shows a coarse preview rather than an empty box,
            // which is the better failure.
            frame.render_widget(HalfBlocks::new(image), inner);
            Some(inner)
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::fixtures;

    /// Draw one frame into an in-memory terminal.
    fn render(app: &App, width: u16, height: u16) -> (String, Option<Rect>) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut area = None;
        terminal.draw(|frame| area = draw(frame, app)).unwrap();

        let buffer = terminal.backend().buffer();
        let text = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        (text, area)
    }

    #[test]
    fn a_frame_shows_every_pane_and_the_key_hints() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        let (text, _) = render(&app, 100, 30);

        for expected in [
            "prompt", "palette", "variants", "renders", "preview", "quit",
        ] {
            assert!(text.contains(expected), "{expected} missing from:\n{text}");
        }
    }

    #[test]
    fn a_ready_preview_reports_the_area_a_graphics_backend_should_draw_into() {
        let mut app = App::new(fixtures::project(), "kitty");
        app.refresh_preview();
        let (_, area) = render(&app, 100, 30);

        let area = area.expect("a ready preview has an area");
        assert!(area.width > 0 && area.height > 0);
        // Inside the border, never over it.
        assert!(area.x > 38, "preview must sit in the right-hand column");
    }

    #[test]
    fn a_failed_preview_reports_no_area_and_shows_the_reason() {
        // A backend must not place an image on top of an error message.
        let mut app = App::new(fixtures::project(), "kitty");
        app.project.metadata_mut().variants.clear();
        app.invalidate_preview();
        app.refresh_preview();

        let (text, area) = render(&app, 100, 30);
        assert!(area.is_none());
        assert!(text.contains("no variants"), "{text}");
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
