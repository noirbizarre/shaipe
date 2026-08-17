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

use super::app::{App, Focus};

/// The border for a pane, highlighted when it has the keyboard.
pub fn frame(title: Focus, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::default().fg(Color::LightMagenta)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style)
        .title(Span::styled(
            format!(" {} ", title.title()),
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
    let text = app.project.metadata().prompt.clone().unwrap_or_else(|| {
        "No prompt recorded.\n\nAdd a <shaipe:prompt> to the project metadata.".to_owned()
    });

    Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Gray))
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
pub fn status(app: &App) -> Paragraph<'static> {
    let hint = |key: &str, action: &str| {
        vec![
            Span::styled(
                key.to_owned(),
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                // Two spaces rather than three: editing and saving added keys
                // to a line that already only just fitted eighty columns, and
                // a hint truncated away is worth nothing.
                format!(" {action}  "),
                Style::default().fg(Color::DarkGray),
            ),
        ]
    };

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
    else if let Some(notice) = &app.notice {
        spans.push(Span::styled(
            notice.clone(),
            Style::default().fg(Color::LightGreen),
        ));
        spans.push(Span::raw("   "));
    } else if app.is_editing() {
        // The workspace's own hints are all wrong while the editor has the
        // keyboard, and listing keys that do something else would be worse
        // than listing none.
        spans.extend(hint("esc", "leave"));
        spans.extend(hint("tab", "pane"));
        spans.extend(hint("ctrl-s", "save"));
    } else {
        spans.extend(hint("tab", "pane"));
        // Which keys are listed follows the focus, for room and for honesty.
        // The prompt has no rows, so `↑↓` never moved anything there; and its
        // own two keys are the only way either is discovered. Re-rendering is
        // dropped from that list because a prompt is metadata — editing it
        // cannot change what the preview shows.
        if app.focus == Focus::Prompt {
            spans.extend(hint("enter", "edit"));
            spans.extend(hint("e", "$EDITOR"));
            spans.extend(hint("ctrl-s", "save"));
        } else {
            spans.extend(hint("↑↓", "select"));
            spans.extend(hint("ctrl-s", "save"));
            spans.extend(hint("r", "render"));
        }
        spans.extend(hint("q", "quit"));
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

    spans.push(Span::styled(
        format!("preview: {}", app.backend),
        Style::default().fg(Color::DarkGray),
    ));

    Paragraph::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;

    use super::*;
    use crate::fixtures;

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

        let output = drawn(status(&app), 120, 1);

        assert!(output.contains("esc"), "{output}");
        assert!(output.contains("leave"), "{output}");
    }

    #[test]
    fn the_prompt_pane_advertises_the_two_keys_only_it_has() {
        // Nothing else in the workspace would ever reveal them.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Prompt;

        let output = drawn(status(&app), 120, 1);

        assert!(output.contains("edit"), "{output}");
        assert!(output.contains("$EDITOR"), "{output}");
    }

    #[test]
    fn the_hints_fit_eighty_columns_even_with_unsaved_work_to_report() {
        // The status line is the one place every key is discoverable, and a
        // hint truncated away is worth nothing.
        for focus in Focus::ALL {
            let mut app = App::new(fixtures::project(), "blocks");
            app.focus = focus;
            app.set_draft("unsaved");

            let output = drawn(status(&app), 80, 1);
            assert!(
                output.contains("preview: blocks"),
                "{focus:?} overflows:\n{output}"
            );
        }
    }

    #[test]
    fn the_status_line_says_when_there_is_unsaved_work() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.set_draft("changed");

        assert!(drawn(status(&app), 120, 1).contains("unsaved"));
    }

    #[test]
    fn the_status_line_asks_before_unsaved_work_is_discarded() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.set_draft("changed");
        app.request_quit();

        let output = drawn(status(&app), 120, 1);
        assert!(output.contains("q again to discard"), "{output}");
    }

    #[test]
    fn a_notice_replaces_the_key_hints_so_it_is_actually_read() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.notice = Some("wrote dist/favicon-32.png".to_owned());
        let output = drawn(status(&app), 90, 1);

        assert!(output.contains("wrote dist/favicon-32.png"), "{output}");
        assert!(output.contains("preview: blocks"), "{output}");
    }

    #[test]
    fn the_status_line_names_the_preview_backend_in_use() {
        // A preview that looks wrong is much easier to report when the user
        // can see which backend drew it.
        let app = App::new(fixtures::project(), "kitty");
        assert!(drawn(status(&app), 80, 1).contains("preview: kitty"));
    }
}
