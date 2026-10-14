//! Where the panes go.
//!
//! ```text
//! ┌ Shaipe  [Chat] [Source] [Renders] [Edit variants] ──────────────────────┐
//! │ description │ visual evidence │ agent chat log (when open)             │
//! │ palette     │                 │                                         │
//! │ references  │                 ├─────────────────────────────────────────┤
//! │             │                 │ transient message composer              │
//! └─────────────┴─────────────────┴─────────────────────────────────────────┘
//! ```
//!
//! The preview draws through [`crate::preview::Preview`], which is a widget
//! like everything else here, so this module never touches an escape
//! sequence and never needs to know which protocol is in use.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::preview::Preview as Backend;

use super::app::{App, Destination, EvidenceView, Focus, NARROW_WIDTH, Preview, View};
use super::history::{Action, Decision};
use super::modal::Modal;
use super::{modal, panes, toolbar};

/// Draw a frame.
pub fn draw(frame: &mut Frame<'_>, app: &mut App, backend: &mut Backend) {
    let [bar, body, status] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(frame.area());

    app.clear_areas(body.width);
    toolbar::draw(frame, app, bar);

    if body.width < NARROW_WIDTH {
        match app.destination() {
            Destination::Evidence => draw_right(frame, app, backend, body),
            Destination::Description => draw_left(frame, app, body),
            Destination::Chat => draw_chat(frame, app, body),
        }
    } else {
        let chat_width = if app.chat_open() {
            app.chat_width(body.width)
        } else {
            0
        };
        let [main, chat] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(body.width.saturating_sub(chat_width)),
                Constraint::Length(chat_width),
            ])
            .areas(body);
        let [left, right] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(app.column(main.width)),
                Constraint::Min(crate::tui::app::MIN_PREVIEW),
            ])
            .areas(main);
        app.set_dividers(
            Rect::new(right.x, body.y, 1, body.height),
            if app.chat_open() {
                Rect::new(chat.x, body.y, 1, body.height)
            } else {
                Rect::ZERO
            },
        );
        draw_left(frame, app, left);
        draw_right(frame, app, backend, right);
        if app.chat_open() {
            draw_chat(frame, app, chat);
        }
    }

    frame.render_widget(panes::status(app, status.width), status);

    // Last, and over everything: a modal that drew under a pane would be a
    // dialogue nobody could read.
    match app.modal() {
        Some(Modal::Variants(editor)) => modal::draw_variants(frame, editor, &app.project, body),
        Some(Modal::Renders(editor)) => modal::draw_renders(frame, editor, &app.project, body),
        Some(Modal::References(editor)) => {
            modal::draw_references(frame, editor, &app.project, body);
        }
        Some(Modal::ColourPicker(picker)) => {
            modal::draw_colour_picker(frame, picker, &app.project, body);
        }
        Some(Modal::Model(picker)) => modal::draw_model_picker(frame, picker, body),
        None => {}
    }
}

/// Draw the description column.
///
/// The description and its project lists remain visible when chat opens.
fn draw_left(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let [top, middle, bottom] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .areas(area);

    app.set_area(Focus::Prompt, top);
    app.set_area(Focus::Palette, middle);
    app.set_area(Focus::References, bottom);

    draw_prompt(frame, app, top);
    draw_palette(frame, app, middle);
    draw_references(frame, app, bottom);
}

fn draw_chat(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let [log, message] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Percentage(25)])
        .areas(area);
    app.set_area(Focus::ChatLog, log);
    app.set_area(Focus::Message, message);
    draw_log(frame, app, log);
    draw_message(frame, app, message);
}

/// The message always has its own session-only buffer.
fn draw_message(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let focused = app.focus == Focus::Message;
    let block = panes::frame_titled(Focus::Message, focused, "message", &panes::agent_title(app));
    let interior = block.inner(area);
    frame.render_widget(block, area);
    if interior.height == 0 {
        return;
    }
    if app.is_editing_message() {
        frame.render_widget(app.message_editor(), interior);
    } else {
        frame.render_widget(panes::message(app), interior);
    }
}

/// Draw the durable project description.
fn draw_prompt(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Prompt;

    // The frame is drawn separately and its interior filled in, rather than
    // the editor carrying a block of its own: a block would have to be set on
    // the text area, and that mutable borrow cannot coexist with the immutable
    // one the contents take below.
    let block = panes::frame_titled(Focus::Prompt, focused, "description", "");
    let interior = block.inner(area);
    frame.render_widget(&block, area);
    if interior.height == 0 {
        return;
    }

    if app.is_editing_prompt() {
        frame.render_widget(app.editor(), interior);
    } else {
        frame.render_widget(panes::prompt(app), interior);
    }
}

fn draw_log(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let focused = app.focus == Focus::ChatLog;
    let block = panes::frame_titled(
        Focus::ChatLog,
        focused,
        "chat log",
        &panes::agent_title(app),
    );
    let interior = block.inner(area);
    frame.render_widget(block, area);
    if interior.width == 0 || interior.height == 0 {
        return;
    }
    let paragraph = Paragraph::new(panes::transcript_lines(app)).wrap(Wrap { trim: false });
    // A long wrapped message occupies more than one row; scroll in rendered
    // rows so the latest response stays in view until the reader scrolls back.
    let rows = u16::try_from(paragraph.line_count(interior.width)).unwrap_or(u16::MAX);
    let bottom = rows.saturating_sub(interior.height);
    frame.render_widget(
        paragraph.scroll((bottom.saturating_sub(app.transcript_scroll()), 0)),
        interior,
    );
}

/// Draw the palette pane.
fn draw_palette(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Palette;
    // Built before the mutable borrow the list state needs.
    let list = panes::palette(&app.project.metadata().palette, focused, app.palette_edit())
        .block(panes::frame(Focus::Palette, focused));
    frame.render_stateful_widget(list, area, app.list_state(Focus::Palette));
}

/// Draw the references pane.
fn draw_references(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::References;
    // Built before the mutable borrow the list state needs, same as the
    // palette above — resolving each `src` needs the whole project, not just
    // its references, so the pane function takes it directly.
    let list =
        panes::references(&app.project, focused).block(panes::frame(Focus::References, focused));
    frame.render_stateful_widget(list, area, app.list_state(Focus::References));
}

/// Draw the preview column: a row of tabs, and whatever they select.
fn draw_right(frame: &mut Frame<'_>, app: &mut App, backend: &mut Backend, area: Rect) {
    let [bar, history, body] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(area);

    draw_tabs(frame, app, bar);
    draw_history(frame, app, history);

    // Told about the area either way, so the preview stays correctly sized
    // and switching back to it is instant rather than a re-render.
    let [image_area, metrics_area] =
        if app.view() == View::Preview && app.display_has_source() && body.height >= 19 {
            Layout::vertical([Constraint::Min(8), Constraint::Length(9)]).areas(body)
        } else {
            [body, Rect::ZERO]
        };
    app.set_preview_area(image_area, backend.cell_size(), backend.scale());
    match app.view() {
        View::Preview => {
            draw_preview(frame, app, backend, image_area);
            if metrics_area.height > 0 {
                draw_metrics(frame, app, metrics_area);
            }
        }
        View::Source => draw_source(frame, app, body),
    }
}

fn draw_history(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let actions = [
        (Action::Previous, "{"),
        (Action::Next, "}"),
        (Action::Live, "H"),
        (Action::Approve, "A✓"),
        (Action::Reject, "J✗"),
        (Action::Restore, "U restore"),
    ];
    let mut spans = Vec::new();
    let mut buttons = Vec::new();
    let mut x = area.x;
    for (action, label) in actions {
        let text = format!(" {label} ");
        let width = text.chars().count() as u16;
        let rect = Rect::new(x, area.y, width, 1).intersection(area);
        if rect.width == width {
            buttons.push((action, rect));
        }
        spans.push(Span::styled(text, Style::default().fg(Color::LightCyan)));
        x = x.saturating_add(width);
    }
    let count = app.history.entries().len();
    let selection = app.history_selected();
    let label = selection.map_or_else(
        || {
            let status =
                app.history
                    .entries()
                    .back()
                    .map_or("pending", |entry| match entry.decision {
                        Decision::Pending => "pending",
                        Decision::Approved => "approved",
                        Decision::Rejected => "rejected",
                    });
            format!(" live · {count} iterations · {status}")
        },
        |entry| {
            let status = match entry.decision {
                Decision::Pending => "pending",
                Decision::Approved => "approved",
                Decision::Rejected => "rejected",
            };
            format!(
                " #{}/{} {status} · {:?}/{:?} · {}",
                entry.id, count, entry.kind, entry.phase, entry.origin
            )
        },
    );
    spans.push(Span::styled(
        label,
        Style::default().fg(Color::LightMagenta),
    ));
    app.set_history_buttons(buttons);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Draw the tab bar, recording where each tab landed.
///
/// The variants or the render specifications, depending on the mode. They were
/// two panes in the left column, which meant the whole of a project's output
/// was described in five rows nobody could read while the artwork had the
/// screen.
fn draw_tabs(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let names = app.tabs();
    let selected = app.tab();

    let mut spans = Vec::new();
    let mut areas = Vec::new();
    let mut x = area.x;

    for (index, name) in names.iter().enumerate() {
        let label = format!(" {name} ");
        let width = label.chars().count() as u16;
        spans.push(Span::styled(
            label,
            if index == selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
        areas.push(Rect::new(x, area.y, width, 1).intersection(area));
        x = x.saturating_add(width);
    }

    if names.is_empty() {
        spans.push(Span::styled(
            " this project declares none ",
            Style::default().fg(Color::DarkGray),
        ));
    }

    app.set_tab_areas(areas);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Draw the document that produced the artwork.
fn draw_source(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " source ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    // Not wrapped: an SVG has meaningful line structure, and rewrapping a
    // path's coordinates across a narrow column makes it unreadable. It runs
    // off the edge instead, which is the lesser harm.
    let paragraph = Paragraph::new(app.display_source()).style(Style::default().fg(Color::Gray));
    frame.render_widget(paragraph.scroll((app.view_scroll(), 0)), inner);
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
    let caption = if let Some(entry) = app.history_selected() {
        format!(
            " {} — iteration #{} · {} ",
            app.evidence_view().title(),
            entry.id,
            entry.variant.as_deref().unwrap_or("no variant")
        )
    } else {
        format!(" {} —{caption}", app.evidence_view().title())
    };
    // In the title rather than over the image: a render can take a noticeable
    // moment, and replacing the previous preview with a spinner would be a
    // downgrade. The old image is more useful than an empty pane.
    let caption = match panes::rendering(app) {
        Some(activity) => format!("{caption}{activity} "),
        None => caption,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            caption,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(entry) = app.history_selected() {
        if let Some(image) = app.display_image() {
            if !app.is_holding_image() {
                backend.draw(image, app.preview_generation(), inner, frame);
            }
        } else {
            frame.render_widget(
                Paragraph::new(entry.error.as_deref().unwrap_or(if entry.pending {
                    "preparing iteration evidence…"
                } else {
                    "this iteration has no image for this view"
                }))
                .wrap(Wrap { trim: false }),
                inner,
            );
        }
        return;
    }

    if app.evidence_view() != EvidenceView::Current {
        if (app.evidence_view() != EvidenceView::Reference || app.evidence_image().is_none())
            && let Some(error) = app.evidence_error()
        {
            frame.render_widget(
                Paragraph::new(format!("comparison unavailable: {error}"))
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(Color::LightRed)),
                inner,
            );
        } else if let Some(image) = app.evidence_image() {
            if app.is_holding_image() {
                frame.render_widget(Paragraph::new("sending to the terminal…"), inner);
            } else {
                backend.draw(image, app.preview_generation(), inner, frame);
            }
        } else {
            frame.render_widget(
                Paragraph::new(if app.evidence_pending() {
                    "preparing reference evidence…"
                } else {
                    "select a variant to compare with this reference"
                }),
                inner,
            );
        }
        return;
    }
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

/// Named measurements from the same comparison that produced the images.
fn draw_metrics(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" comparison ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(report) = app.display_comparison() else {
        let message = app.display_error().unwrap_or(if app.evidence_pending() {
            "computing comparison…"
        } else {
            "select a variant to compare"
        });
        frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), inner);
        return;
    };
    let (source, variant) = app
        .history_selected()
        .and_then(|entry| {
            entry
                .evidence
                .as_ref()
                .map(|e| (e.source.as_path(), e.variant.as_deref()))
        })
        .or_else(|| app.comparison_context())
        .unwrap_or((std::path::Path::new(""), None));
    let text = format!(
        "{} ↔ {}  {}×{}\nforeground overlap {:.1}% · edges {:.1}%\nchanged pixels {:.1}% · MAE {:.3}\nperceptual similarity {:.3} ({})\ncentroid offset {}\nappearance: {} · lettering: {}",
        source.display(),
        variant.unwrap_or("(none)"),
        report.canvas.width,
        report.canvas.height,
        report.foreground.intersection_over_union * 100.0,
        report.edges.intersection_over_union * 100.0,
        report.pixel_error.changed_pixel_fraction * 100.0,
        report.pixel_error.mean_absolute_error,
        report.perceptual_similarity.score,
        report.perceptual_similarity.algorithm,
        report.centroid.offset.map_or_else(
            || "n/a".to_owned(),
            |offset| format!("{:+.1}, {:+.1} px", offset.dx, offset.dy)
        ),
        report
            .appearance
            .findings
            .first()
            .map_or("no findings", String::as_str),
        report
            .typography
            .findings
            .first()
            .map_or("no findings", String::as_str),
    );
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::fixtures;
    use crate::preview::Backend as PreviewBackend;
    use crate::tui::app::Mode;

    #[test]
    fn history_controls_and_candidate_status_are_visible_and_clickable_on_a_narrow_screen() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.set_draft("a newly edited description");
        app.previous_iteration();
        app.review_iteration(Decision::Rejected);
        let text = render(&mut app, 50, 25);
        assert!(text.contains("U restore"), "{text}");
        assert!(text.contains("rejected"), "{text}");
        assert!((0..50).any(|x| app.history_button_at(x, 2) == Some(Action::Restore)));
        assert!(text.contains("#1/2"), "{text}");
    }

    #[test]
    fn inspecting_a_candidate_shows_its_frozen_reference_and_comparison_after_live_edits() {
        let dir = tempfile::tempdir().unwrap();
        let mut project =
            crate::Project::from_source(dir.path().join("logo.svg"), fixtures::PROJECT.to_owned())
                .unwrap();
        project
            .metadata_mut()
            .references
            .push(crate::project::Reference::new(
                "source.png",
                crate::project::ReferenceKind::Source,
            ));
        let white = crate::preview::Image::from_rgba(32, 32, vec![255; 32 * 32 * 4]);
        std::fs::write(dir.path().join("source.png"), white.to_png().unwrap()).unwrap();
        let mut app = App::new(project, "blocks");
        app.history.wait();
        app.set_draft("different live intent");
        app.previous_iteration();
        app.cycle_evidence();
        assert_eq!(app.evidence_view(), EvidenceView::Reference);
        assert_eq!(app.display_image().unwrap().pixel(0, 0), [255; 4]);
        let text = render(&mut app, 120, 36);
        assert!(text.contains("foreground overlap"), "{text}");
        assert!(text.contains("source.png"), "{text}");
        assert!(painted(&frame(&mut app, 120, 36), app.preview_area()) > 0);
        app.cycle_evidence();
        assert!(
            app.display_image().is_some(),
            "overlay is retained for review"
        );
        app.cycle_evidence();
        assert!(
            app.display_image().is_some(),
            "difference is retained for review"
        );
        assert_eq!(
            app.project.metadata().description.as_deref(),
            Some("different live intent")
        );
    }

    #[test]
    fn reference_and_difference_views_show_their_pixels_with_named_measurements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logo.svg");
        let mut project = crate::Project::from_source(path, fixtures::PROJECT.to_owned()).unwrap();
        project
            .metadata_mut()
            .references
            .push(crate::project::Reference::new(
                "source.png",
                crate::project::ReferenceKind::Source,
            ));
        let image = crate::preview::Image::from_rgba(32, 32, vec![255; 32 * 32 * 4]);
        std::fs::write(dir.path().join("source.png"), image.to_png().unwrap()).unwrap();

        let mut app = App::new(project, "blocks");
        app.next_workflow_kind();
        app.next_workflow_kind();
        app.refresh_evidence();
        app.cycle_evidence();
        let reference = render(&mut app, 120, 36);
        assert!(reference.contains("foreground overlap"), "{reference}");
        assert!(reference.contains("source.png"), "{reference}");
        assert!(reference.contains("reference"), "{reference}");
        app.cycle_evidence();
        app.cycle_evidence();
        let difference = render(&mut app, 120, 36);
        assert!(difference.contains("difference"), "{difference}");
        assert!(difference.contains("perceptual similarity"), "{difference}");
        assert!(!app.is_dirty());
    }

    #[test]
    fn from_scratch_shows_no_comparison_placeholder() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.next_workflow_kind();
        let text = render(&mut app, 50, 25);
        assert!(!text.contains("comparison unavailable"), "{text}");
        assert!(!text.contains("heatmap"), "{text}");
    }

    #[test]
    fn a_reference_remains_visible_when_its_variant_cannot_render() {
        let dir = tempfile::tempdir().unwrap();
        let mut project =
            crate::Project::from_source(dir.path().join("logo.svg"), fixtures::PROJECT.to_owned())
                .unwrap();
        project
            .metadata_mut()
            .references
            .push(crate::project::Reference::new(
                "source.png",
                crate::project::ReferenceKind::Source,
            ));
        std::fs::write(
            dir.path().join("source.png"),
            crate::preview::Image::from_rgba(32, 32, vec![255; 32 * 32 * 4])
                .to_png()
                .unwrap(),
        )
        .unwrap();
        project.metadata_mut().variants.clear();
        let mut app = App::new(project, "blocks");
        app.next_workflow_kind();
        app.next_workflow_kind();
        app.refresh_evidence();
        app.cycle_evidence();
        let buffer = frame(&mut app, 120, 36);
        assert!(painted(&buffer, app.preview_area()) > 0);
        let text = render(&mut app, 120, 36);
        assert!(text.contains("select a variant to compare"), "{text}");
    }

    #[test]
    fn a_missing_reference_reports_its_path_in_the_evidence_pane() {
        let dir = tempfile::tempdir().unwrap();
        let mut project =
            crate::Project::from_source(dir.path().join("logo.svg"), fixtures::PROJECT.to_owned())
                .unwrap();
        project
            .metadata_mut()
            .references
            .push(crate::project::Reference::new(
                "missing.png",
                crate::project::ReferenceKind::Source,
            ));
        let mut app = App::new(project, "blocks");
        app.next_workflow_kind();
        app.next_workflow_kind();
        app.refresh_evidence();
        app.cycle_evidence();
        let text = render(&mut app, 120, 36);
        assert!(text.contains("comparison unavailable"), "{text}");
        assert!(text.contains("missing.png"), "{text}");
    }

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
        // palette pane's immutable borrow of the project.
        let mut app = App::new(fixtures::project(), "blocks");
        app.engage_editor();
        app.set_draft("an engaged editor");

        let text = render(&mut app, 100, 30);

        assert!(text.contains("an engaged editor"), "{text}");
        // The rest of the workspace is still drawn around it.
        assert!(text.contains("palette"), "{text}");
    }

    #[test]
    fn a_frame_shows_the_toolbar_the_panes_the_tabs_and_the_key_hints() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        let text = render(&mut app, 100, 30);

        for expected in [
            // The toolbar: the workspace's name, and what each button does.
            "Shaipe",
            "Chat",
            "Source",
            "Renders",
            // The panes that are left.
            "description",
            "palette",
            // A tab per variant, and a way out.
            "icon",
            "quit",
        ] {
            assert!(text.contains(expected), "{expected} missing from:\n{text}");
        }
    }

    #[test]
    fn the_left_column_is_half_the_body_until_somebody_drags_it() {
        // It was thirty-eight characters, which is a sensible width for four
        // lists of names and a poor one for a paragraph of prose.
        let mut app = App::new(fixtures::project(), "blocks");
        frame(&mut app, 100, 30);

        assert_eq!(app.column_width, 50);
        assert_eq!(app.area(Focus::Prompt).width, 50);
    }

    #[test]
    fn the_prompt_keeps_the_largest_share_of_the_column_whatever_has_focus() {
        // Unconditionally: the prompt is what the workspace is about, and
        // making it grow only when the keyboard is in it means it is short
        // exactly when somebody is reading it. The palette and the
        // references pane split what is left evenly.
        let mut app = App::new(fixtures::project(), "blocks");

        app.focus = Focus::References;
        frame(&mut app, 100, 30);

        let prompt = app.area(Focus::Prompt).height;
        let palette = app.area(Focus::Palette).height;
        let references = app.area(Focus::References).height;
        assert!(
            prompt > palette && prompt > references,
            "the prompt should keep the largest share whatever has focus: \
             {prompt} vs {palette} vs {references}"
        );
        assert!(references > 0, "the references pane was not given any room");
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
    fn a_long_prompt_is_read_from_the_top() {
        // Not bottom-anchored: a prompt is a thing you wrote, not a
        // conversation with a newest end.
        let mut app = App::new(fixtures::project(), "blocks");
        app.project.metadata_mut().description = Some(LONG_PROMPT.to_owned());

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("A mark for Shaipe"),
            "the prompt did not start at the beginning:\n{text}"
        );
    }

    #[test]
    fn the_transcript_shows_its_newest_entries_even_when_they_wrap() {
        // The regression test for the whole business. The old one passed only
        // because the fixture's prompt was one row long, so counting entries
        // happened to equal counting rows. Here every entry wraps to several
        // rows, and an offset measured in entries lands nowhere near the end.
        let mut app = App::new(fixtures::project(), "blocks");

        for turn in 0..12 {
            app.transcript
                .push_user(format!("{LONG_PROMPT} — asked{turn}"));
            app.transcript
                .apply(crate::acp::AgentUpdate::Message(format!(
                    "{LONG_PROMPT} — answered{turn}"
                )));
        }
        app.toggle_chat();

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
    fn the_transcript_says_what_to_do_when_it_is_empty() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.toggle_chat();

        let text = render(&mut app, 100, 30);
        assert!(text.contains("Nothing yet"), "{text}");
    }

    #[test]
    fn opening_chat_keeps_the_description_and_evidence_visible() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.transcript
            .push_user("make the tail more angular".to_owned());
        let original = app.project.metadata().description.clone();
        app.toggle_chat();

        let text = render(&mut app, 120, 30);
        for label in [
            "description",
            "A square and a bar",
            "preview",
            "chat log",
            "more angular",
            "message",
        ] {
            assert!(text.contains(label), "{label} missing from:\n{text}");
        }
        assert_eq!(
            app.focus,
            Focus::Prompt,
            "opening chat must not steal focus"
        );
        assert_eq!(app.project.metadata().description, original);
        assert!(!app.is_dirty());
        assert!(app.area(Focus::ChatLog).width >= 24);
    }

    #[test]
    fn a_narrow_screen_has_separate_evidence_description_and_chat_destinations() {
        let mut app = App::new(fixtures::project(), "blocks");
        let evidence = render(&mut app, 70, 26);
        assert!(evidence.contains("preview"), "{evidence}");
        assert_eq!(app.area(Focus::Prompt), Rect::ZERO);

        app.show_destination(Destination::Description);
        let description = render(&mut app, 70, 26);
        assert!(description.contains("A square and a bar"), "{description}");
        assert_eq!(app.area(Focus::ChatLog), Rect::ZERO);

        app.toggle_chat();
        let chat = render(&mut app, 70, 26);
        assert!(chat.contains("chat log"), "{chat}");
        assert!(chat.contains("message"), "{chat}");
        assert_eq!(app.area(Focus::Prompt), Rect::ZERO);
        assert_eq!(app.pane_at(2, 3), Some(Focus::ChatLog));

        app.focus_next();
        assert_eq!(app.focus, Focus::ChatLog);
        app.focus_next();
        assert_eq!(app.focus, Focus::Message);
        app.show_destination(Destination::Description);
        assert_eq!(app.focus, Focus::Prompt);
        assert_eq!(
            app.area(Focus::Prompt),
            Rect::ZERO,
            "no draw has happened yet"
        );
        render(&mut app, 70, 26);
        assert_eq!(
            app.area(Focus::ChatLog),
            Rect::ZERO,
            "hidden panes are not clickable"
        );
    }

    #[test]
    fn the_prompt_pane_says_why_there_is_no_agent() {
        // The diagnostic belongs where the person who cannot ask for anything
        // is looking. Said before a message is typed and swallowed, because
        // that reads like a bug rather than like a missing dependency.
        let mut app = App::new(fixtures::project(), "blocks");
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
    ///
    /// One row of toolbar and one of tabs above it now, and the column starts
    /// halfway across.
    const PREVIEW: Rect = Rect {
        x: 51,
        y: 3,
        width: 48,
        height: 25,
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
    fn the_tabs_list_the_variants_or_the_specifications_depending_on_the_mode() {
        // They were two panes in the left column, which described the whole of
        // a project's output in five rows nobody could read.
        let mut app = App::new(fixtures::project(), "blocks");

        let variants = render(&mut app, 100, 30);
        assert!(variants.contains("icon"), "{variants}");
        assert!(variants.contains("wordmark"), "{variants}");

        app.toggle_mode();
        assert_eq!(app.mode(), Mode::Renders);
        let renders = render(&mut app, 100, 30);
        assert!(renders.contains("favicon-32"), "{renders}");
    }

    #[test]
    fn the_selected_tab_is_the_one_that_is_marked() {
        let mut app = App::new(fixtures::project(), "blocks");
        frame(&mut app, 100, 30);

        // Wherever the layout put them, every tab is clickable and the first
        // cell of the bar belongs to the first tab.
        let first = (0..30)
            .flat_map(|row| (0..100).map(move |column| (column, row)))
            .find_map(|(column, row)| app.tab_at(column, row));
        assert_eq!(first, Some(0), "the tabs were not drawn anywhere");

        app.next_tab();
        assert_eq!(app.tab(), 1);
    }

    #[test]
    fn the_source_view_replaces_the_picture_rather_than_sitting_beside_it() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        app.toggle_view();

        let rendered = frame(&mut app, 100, 30);
        let text = (0..30)
            .map(|y| {
                (0..100)
                    .map(|x| rendered[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("<svg"), "{text}");
        assert_eq!(painted(&rendered, PREVIEW), 0, "the image is still drawn");
    }

    #[test]
    fn the_renders_editor_covers_the_workspace() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        app.open_renders_editor();

        let rendered = frame(&mut app, 100, 30);
        let rows: Vec<String> = (0..30)
            .map(|y| {
                (0..100)
                    .map(|x| rendered[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        let text = rows.join("\n");

        assert!(text.contains("render specifications"), "{text}");
        assert!(text.contains("background"), "the table's columns\n{text}");

        // Over the picture, not beside it: the table is wider than the left
        // column, so its last heading lands in the preview's own columns and
        // there is no image left underneath it.
        let (row, column) = rows
            .iter()
            .enumerate()
            .find_map(|(row, line)| line.find("background").map(|column| (row, column)))
            .expect("the heading is on some row");
        let (row, column) = (u16::try_from(row).unwrap(), u16::try_from(column).unwrap());
        assert!(
            column > PREVIEW.x,
            "the dialogue did not reach the preview's columns"
        );
        assert_eq!(
            painted(
                &rendered,
                Rect::new(column, row, "background".len() as u16, 1)
            ),
            0,
            "the picture was drawn through the dialogue"
        );
    }

    #[test]
    fn the_variants_editor_covers_the_workspace() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.refresh_preview();
        app.open_variants_editor();

        let text = render(&mut app, 100, 30);

        assert!(text.contains("variants"), "{text}");
        assert!(text.contains("element"), "the table's columns\n{text}");
        // The fixture's own variants, both on the table at once.
        assert!(text.contains("icon"), "{text}");
        assert!(text.contains("wordmark"), "{text}");
    }

    #[test]
    fn the_colour_picker_shows_the_selected_colours_name_and_its_controls() {
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Palette;
        app.open_colour_picker();
        assert!(app.modal().is_some());

        let text = render(&mut app, 100, 30);

        // The fixture's first colour, and the sliders a value is turned with.
        assert!(text.contains("accent"), "{text}");
        assert!(text.contains("hue"), "{text}");
        assert!(text.contains("saturation"), "{text}");
        assert!(text.contains("lightness"), "{text}");
        assert!(text.contains("alpha"), "{text}");
        assert!(text.contains("hex"), "{text}");
    }

    #[test]
    fn a_palette_being_edited_shows_what_has_been_typed_rather_than_what_is_committed() {
        // Otherwise the keystrokes are invisible until they happen to parse,
        // which for a hex colour is only at the very end.
        let mut app = App::new(fixtures::project(), "blocks");
        app.focus = Focus::Palette;
        app.engage_editor();
        for _ in 0..7 {
            app.edit_key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Backspace,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ));
        }
        for character in "#0066".chars() {
            app.edit_key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char(character),
                ratatui::crossterm::event::KeyModifiers::NONE,
            ));
        }

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("#0066"),
            "the half-typed colour is hidden\n{text}"
        );
    }

    #[test]
    fn a_pane_smaller_than_its_contents_scrolls_to_keep_the_selection_visible() {
        // The palette is the one list left, and it can outgrow a third of the
        // column; without this the selection could sit off-screen.
        let mut app = App::new(fixtures::project(), "blocks");
        for index in 0..20 {
            app.project
                .metadata_mut()
                .palette
                .push(crate::project::Colour {
                    name: format!("colour-{index}"),
                    value: crate::project::Rgba::new(0x11, 0x22, 0x33, 0xff),
                    role: None,
                });
        }

        app.focus = Focus::Palette;
        while app.selection(Focus::Palette) + 1 < app.project.metadata().palette.len() {
            app.select_next();
        }

        let text = render(&mut app, 100, 30);
        assert!(
            text.contains("colour-19"),
            "the selected entry should have scrolled into view:\n{text}"
        );
    }

    #[test]
    fn a_running_render_puts_a_spinner_on_the_screen() {
        // The reported symptom was an empty preview box with no sign that
        // anything was happening. It has to reach the frame.
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

        // And with the dialogue up, which is the one thing drawn over the top.
        app.open_renders_editor();
        for (width, height) in [(1, 1), (5, 3), (40, 2), (200, 60)] {
            render(&mut app, width, height);
        }

        // The model picker lays out a search field, a list and a hint line
        // inside whatever room the dialogue gets — the one modal with more
        // than one row of internal layout to overflow.
        app.close_modal();
        app.set_models(vec![crate::acp::ModelChoice {
            id: "opencode/grok-code".to_owned(),
            name: "Grok Code".to_owned(),
            current: true,
        }]);
        app.open_model_picker();
        for (width, height) in [(1, 1), (5, 3), (40, 2), (200, 60)] {
            render(&mut app, width, height);
        }
    }
}
