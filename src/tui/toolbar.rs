//! The row of controls along the top.
//!
//! The workspace's modes used to be reachable only by keys nobody could
//! discover from the screen: which of the artwork and the source is shown,
//! which of the variants and the render specifications the tabs list, and
//! whether the transcript is up. Each of those is a state, and a state with no
//! visible affordance is a state people find by accident.
//!
//! Every button is built once, into a list carrying the rectangle it was drawn
//! in, and both the drawing and the hit-testing read that list. Two copies of
//! the arithmetic would drift, and the bug that caused would be a click landing
//! on the wrong control.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::app::{App, LeftView, Mode, View};

/// What a toolbar button does.
///
/// The button, not the keystroke: a click and the key that does the same thing
/// go through one function, so the two can never disagree about what a control
/// means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Swap the artwork for the document that produced it.
    View,
    /// Swap the variants for the render specifications.
    Mode,
    /// Open the render specifications editor.
    Renders,
    /// Swap the prompt for the transcript.
    Transcript,
}

/// How the workspace is named on its own toolbar.
const NAME: &str = " shaipe ";

/// One button, as it will be drawn.
struct Segment {
    button: Button,
    label: String,
    active: bool,
}

/// The buttons, left to right, with what each currently says.
///
/// A toggle is labelled with the state it is *in*, not the one it would move
/// to. A button reading "source" while the source is on screen is ambiguous
/// exactly once per user, and they resolve it by pressing it and losing their
/// place.
fn segments(app: &App) -> Vec<Segment> {
    vec![
        Segment {
            button: Button::View,
            label: format!(" {} ", app.view().title()),
            active: app.view() == View::Source,
        },
        Segment {
            button: Button::Mode,
            label: format!(" {} ", app.mode().title()),
            active: app.mode() == Mode::Renders,
        },
        Segment {
            button: Button::Renders,
            label: " edit renders ".to_owned(),
            active: app.modal().is_some(),
        },
        Segment {
            button: Button::Transcript,
            label: " transcript ".to_owned(),
            active: app.left_view() == LeftView::Transcript,
        },
    ]
}

/// Draw the toolbar, recording where each button landed.
pub fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let mut spans = vec![Span::styled(
        NAME,
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD),
    )];
    // The name is not a button, so its columns are not clickable.
    let mut x = area.x.saturating_add(NAME.len() as u16);
    let mut buttons = Vec::new();

    for segment in segments(app) {
        spans.push(Span::raw(" "));
        x = x.saturating_add(1);

        let width = segment.label.chars().count() as u16;
        spans.push(Span::styled(
            segment.label,
            if segment.active {
                Style::default().fg(Color::Black).bg(Color::LightCyan)
            } else {
                Style::default().fg(Color::Gray).bg(Color::DarkGray)
            },
        ));

        // Recorded even when it runs off the edge: `Rect` intersected with a
        // narrow terminal simply contains no point, so a button nobody can see
        // is a button nobody can click.
        buttons.push((
            segment.button,
            Rect::new(x, area.y, width, 1).intersection(area),
        ));
        x = x.saturating_add(width);
    }

    app.set_toolbar(buttons);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::fixtures;

    /// Draw a toolbar into an in-memory terminal and read it back.
    fn drawn(app: &mut App, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| draw(frame, app, Rect::new(0, 0, width, 1)))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..width).map(|x| buffer[(x, 0)].symbol()).collect()
    }

    fn app() -> App {
        App::new(fixtures::project(), "blocks")
    }

    #[test]
    fn the_toolbar_names_the_workspace_and_every_state_it_is_in() {
        let mut app = app();
        let line = drawn(&mut app, 80);

        assert!(line.contains("shaipe"), "{line}");
        assert!(line.contains("preview"), "{line}");
        assert!(line.contains("variants"), "{line}");
        assert!(line.contains("transcript"), "{line}");
    }

    #[test]
    fn a_toggle_is_labelled_with_the_state_it_is_in_not_the_one_it_moves_to() {
        // Ambiguous exactly once per user, and they resolve it by pressing the
        // button and losing their place.
        let mut app = app();
        assert!(drawn(&mut app, 80).contains("preview"));

        app.toggle_view();
        let line = drawn(&mut app, 80);
        assert!(line.contains("source"), "{line}");
        assert!(!line.contains("preview"), "{line}");
    }

    #[test]
    fn a_click_lands_on_the_button_that_was_drawn_under_it() {
        // The whole reason the areas are recorded while drawing rather than
        // recomputed when a click arrives.
        let mut app = app();
        drawn(&mut app, 80);

        for expected in [
            Button::View,
            Button::Mode,
            Button::Renders,
            Button::Transcript,
        ] {
            let area = app
                .button_area(expected)
                .unwrap_or_else(|| panic!("{expected:?} was not drawn"));
            assert_eq!(app.button_at(area.x, area.y), Some(expected));
        }
    }

    #[test]
    fn nothing_outside_a_button_is_clickable() {
        let mut app = app();
        drawn(&mut app, 80);

        // The workspace's own name, and the empty end of the line.
        assert_eq!(app.button_at(1, 0), None);
        assert_eq!(app.button_at(79, 0), None);
    }
}
