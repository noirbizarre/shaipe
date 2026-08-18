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

use super::app::App;

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

/// How the workspace is named on its own toolbar, and how wide that is.
///
/// Three pieces because the name is a pun — "shape" and "AI" — and this is the
/// one place in the product that ever says so.
const NAME: [(&str, bool); 3] = [("Sh", false), ("ai", true), ("pe", false)];

/// How many columns the name occupies.
///
/// Written out rather than summed at runtime: it is three literals, and the
/// arithmetic that matters here is where the first *button* starts.
const NAME_WIDTH: u16 = 6;

/// One button, as it will be drawn.
struct Segment {
    button: Button,
    label: String,
}

/// The buttons, left to right, with what each will do.
///
/// A button is labelled with the state it moves *to*, never the one it is in,
/// and nothing is highlighted. Both halves of that are the same decision: the
/// label already says what pressing it does, so a colour saying it too is
/// redundant — and, being the only colour on the row, it read as "this is the
/// selected one", which is the opposite of what a button that switches away
/// means.
fn segments(app: &App) -> Vec<Segment> {
    vec![
        // The left column is read first, so the button for it comes first.
        Segment {
            button: Button::Transcript,
            label: label(app.left_view().other().title()),
        },
        Segment {
            button: Button::View,
            label: label(app.view().other().title()),
        },
        Segment {
            button: Button::Mode,
            label: label(app.mode().other().title()),
        },
        Segment {
            button: Button::Renders,
            label: label("edit renders"),
        },
    ]
}

/// A button's text: capitalised, and padded off its neighbours.
///
/// Capitalised here rather than in the enums, so that the same words stay
/// lower case in the pane titles and the key hints, where they are prose.
fn label(text: &str) -> String {
    let mut characters = text.chars();
    let capitalised = match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    };
    format!(" {capitalised} ")
}

/// Draw the toolbar, recording where each button landed.
pub fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    // Plain text on the ordinary background. Given one of its own it read as a
    // button, which is a thing it is not: clicking it does nothing, and a
    // control that does nothing is worse than no control.
    let mut spans: Vec<Span<'static>> = NAME
        .iter()
        .map(|(part, bold)| {
            let style = Style::default().fg(Color::LightMagenta);
            Span::styled(
                (*part).to_owned(),
                if *bold {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                },
            )
        })
        .collect();

    // The name is not a button, so its columns are not clickable.
    let mut x = area.x.saturating_add(NAME_WIDTH);
    let mut buttons = Vec::new();

    for segment in segments(app) {
        spans.push(Span::raw(" "));
        x = x.saturating_add(1);

        let width = segment.label.chars().count() as u16;
        spans.push(Span::styled(
            segment.label,
            Style::default().fg(Color::Gray).bg(Color::DarkGray),
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
    fn the_toolbar_names_the_workspace_and_everything_that_can_be_pressed() {
        let mut app = app();
        let line = drawn(&mut app, 80);

        assert!(line.contains("Shaipe"), "{line}");
        for expected in ["Transcript", "Source", "Renders", "Edit renders"] {
            assert!(line.contains(expected), "{expected} missing from\n{line}");
        }
    }

    #[test]
    fn a_button_is_labelled_with_what_a_click_will_do() {
        // Naming the state it is *in* is ambiguous exactly once per user, and
        // they resolve it by pressing the button and losing their place.
        let mut app = app();
        let line = drawn(&mut app, 80);
        assert!(line.contains("Source"), "{line}");
        assert!(!line.contains("Preview"), "{line}");

        app.toggle_view();
        let line = drawn(&mut app, 80);
        assert!(line.contains("Preview"), "{line}");
        assert!(!line.contains("Source"), "{line}");
    }

    #[test]
    fn the_transcript_button_comes_first_and_says_which_way_it_goes() {
        // The left column is read first, so the button for it is read first.
        let mut app = app();
        let line = drawn(&mut app, 80);
        assert!(
            line.find("Transcript") < line.find("Source"),
            "the transcript button is not first:\n{line}"
        );

        app.toggle_left_view();
        let line = drawn(&mut app, 80);
        assert!(line.contains("Prompt"), "{line}");
        assert!(!line.contains("Transcript"), "{line}");
    }

    #[test]
    fn the_workspaces_name_is_not_a_button() {
        // Given a background of its own it read as one — and clicking it does
        // nothing, which is worse than having no control at all.
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut app, Rect::new(0, 0, 80, 1)))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        for x in 0..NAME_WIDTH {
            assert_eq!(
                buffer[(x, 0)].bg,
                Color::Reset,
                "the name is drawn as a button at column {x}"
            );
            assert_eq!(app.button_at(x, 0), None, "column {x} is clickable");
        }
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
