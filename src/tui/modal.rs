//! The modal views, and the overlay they are drawn in.
//!
//! The first thing in this workspace that covers the panes rather than sitting
//! beside them. That is deliberate and it is narrow: a render specification has
//! six fields, and six fields wanted a table, and a table wanted the width of
//! the screen. Everything else the workspace does still happens in place.
//!
//! While a modal is open it owns every key — the caller asks it first and does
//! nothing with what it takes — and every mouse event outside its rectangle is
//! dropped. The panes underneath record their areas as they always did, and a
//! click reaching one of them through the overlay would act on something that
//! is not on screen.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::project::{Background, Format, Project, RenderSpec};

/// What is covering the workspace.
///
/// An enum with one variant, because the second — a palette editor — was
/// considered and rejected: the palette is a pane and is edited in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// The render specifications editor.
    Renders(RendersEditor),
}

/// One column of the specifications table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The output file's stem.
    Name,
    /// Which variant it draws.
    Variant,
    /// Canvas width.
    Width,
    /// Canvas height.
    Height,
    /// How to encode it.
    Format,
    /// What to draw it on.
    Background,
}

impl Field {
    /// Every column, left to right.
    const ALL: [Self; 6] = [
        Self::Name,
        Self::Variant,
        Self::Width,
        Self::Height,
        Self::Format,
        Self::Background,
    ];

    /// The column's heading.
    const fn title(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Variant => "variant",
            Self::Width => "width",
            Self::Height => "height",
            Self::Format => "format",
            Self::Background => "background",
        }
    }

    /// How wide it is drawn, in characters.
    const fn width(self) -> u16 {
        match self {
            Self::Name | Self::Variant => 16,
            Self::Width | Self::Height => 8,
            Self::Format => 8,
            Self::Background => 14,
        }
    }

    /// Its position, for moving between columns.
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or_default()
    }

    /// The next column round.
    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    /// The previous column round.
    fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// What a specification currently says for this column.
    fn read(self, spec: &RenderSpec) -> String {
        match self {
            Self::Name => spec.name.clone(),
            Self::Variant => spec.variant.clone(),
            Self::Width => spec.width.to_string(),
            Self::Height => spec.height.to_string(),
            Self::Format => spec.format.to_string(),
            Self::Background => spec.background.to_string(),
        }
    }

    /// Write `text` back, reporting whether it said anything usable.
    ///
    /// A field that does not parse leaves the specification exactly as it was.
    /// Half of every number and every colour is unparsable while it is being
    /// typed, so refusing the keystroke instead would make the table unusable.
    fn write(self, spec: &mut RenderSpec, text: &str) -> bool {
        let text = text.trim();
        match self {
            // An empty name is a file called `.png`, and an empty variant is a
            // specification that can never render.
            Self::Name if !text.is_empty() => spec.name = text.to_owned(),
            Self::Variant if !text.is_empty() => spec.variant = text.to_owned(),
            // Zero is a canvas the renderer refuses, so it is not a size that
            // can be committed on the way to typing a real one.
            Self::Width => match text.parse::<u32>() {
                Ok(width) if width > 0 => spec.width = width,
                _ => return false,
            },
            Self::Height => match text.parse::<u32>() {
                Ok(height) if height > 0 => spec.height = height,
                _ => return false,
            },
            Self::Format => match text.parse::<Format>() {
                Ok(format) => spec.format = format,
                Err(_) => return false,
            },
            Self::Background => match text.parse::<Background>() {
                Ok(background) => spec.background = background,
                Err(_) => return false,
            },
            _ => return false,
        }
        true
    }
}

/// The size a new specification is added at.
///
/// Square, PNG and transparent, which is the shape of the overwhelming
/// majority of them, and large enough to be worth looking at before it is
/// retyped.
const NEW_SIZE: u32 = 256;

/// The render specifications editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendersEditor {
    row: usize,
    field: Field,
    /// What has been typed into the current cell.
    ///
    /// A buffer rather than the specification itself, for the reason
    /// [`Field::write`] gives: the intermediate states of a number are not
    /// numbers.
    buffer: String,
    closed: bool,
}

impl RendersEditor {
    /// Open the editor on a row.
    #[must_use]
    pub fn new(row: usize, project: &Project) -> Self {
        let mut editor = Self {
            row,
            field: Field::Name,
            buffer: String::new(),
            closed: false,
        };
        editor.load(project);
        editor
    }

    /// Which row is being edited.
    #[must_use]
    pub const fn row(&self) -> usize {
        self.row
    }

    /// Whether the editor has been asked to close.
    #[must_use]
    pub const fn closed(&self) -> bool {
        self.closed
    }

    /// Fill the buffer from the cell now selected.
    fn load(&mut self, project: &Project) {
        self.buffer = project
            .metadata()
            .renders
            .get(self.row)
            .map(|spec| self.field.read(spec))
            .unwrap_or_default();
    }

    /// Write the buffer back, reporting whether the project changed.
    fn commit(&self, project: &mut Project) -> bool {
        let buffer = self.buffer.clone();
        let Some(spec) = project.metadata_mut().renders.get_mut(self.row) else {
            return false;
        };
        let before = spec.clone();
        self.field.write(spec, &buffer);
        *spec != before
    }

    /// Apply a keypress, reporting whether the project changed.
    ///
    /// The arrows walk the table and every one of them commits first, so
    /// leaving a cell is as good as finishing it. `ctrl-n` and `ctrl-d` add and
    /// remove a row: chords rather than `+` and `-`, which are characters
    /// somebody typing a size would expect to reach the buffer.
    pub fn key(&mut self, key: KeyEvent, project: &mut Project) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                let mutated = self.commit(project);
                self.closed = true;
                return mutated;
            }
            KeyCode::Char('n') if control => {
                self.commit(project);
                self.add(project);
                // Always: a row was added, whatever the cell being left said.
                return true;
            }
            KeyCode::Char('d') if control => return self.remove(project),
            KeyCode::Up => {
                let mutated = self.commit(project);
                self.row = self.row.saturating_sub(1);
                self.load(project);
                return mutated;
            }
            KeyCode::Down => {
                let mutated = self.commit(project);
                let last = project.metadata().renders.len().saturating_sub(1);
                self.row = self.row.saturating_add(1).min(last);
                self.load(project);
                return mutated;
            }
            KeyCode::Left | KeyCode::BackTab => {
                let mutated = self.commit(project);
                self.field = self.field.previous();
                self.load(project);
                return mutated;
            }
            KeyCode::Right | KeyCode::Tab | KeyCode::Enter => {
                let mutated = self.commit(project);
                self.field = self.field.next();
                self.load(project);
                return mutated;
            }
            KeyCode::Backspace => {
                self.buffer.pop();
            }
            KeyCode::Char(character) => self.buffer.push(character),
            _ => return false,
        }

        self.commit(project)
    }

    /// Add a specification below the one selected.
    fn add(&mut self, project: &mut Project) {
        let variant = project
            .metadata()
            .variants
            .first()
            .map_or_else(|| "icon".to_owned(), |variant| variant.name.clone());
        // Named after its position rather than left blank: an unnamed
        // specification writes a file called `.png`, and the name is the one
        // field that cannot be left for later.
        let name = format!("render-{}", project.metadata().renders.len() + 1);

        let specs = &mut project.metadata_mut().renders;
        let at = (self.row + 1).min(specs.len());
        specs.insert(at, RenderSpec::square(name, variant, NEW_SIZE));

        self.row = at;
        self.field = Field::Name;
        self.load(project);
    }

    /// Remove the selected specification.
    fn remove(&mut self, project: &mut Project) -> bool {
        let specs = &mut project.metadata_mut().renders;
        if self.row >= specs.len() {
            return false;
        }
        specs.remove(self.row);
        self.row = self.row.min(specs.len().saturating_sub(1));
        self.load(project);
        true
    }
}

/// A rectangle in the middle of `area`, at most `width` by `height`.
///
/// Clamped rather than assumed: terminals get dragged to absurd sizes, and a
/// modal larger than the screen is a subtraction overflow waiting to happen.
#[must_use]
pub fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let [_, middle, _] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height - height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .areas(area);
    let [_, centre, _] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((area.width - width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .areas(middle);
    centre
}

/// Draw the render specifications editor over the workspace.
pub fn draw(frame: &mut Frame<'_>, editor: &RendersEditor, project: &Project, area: Rect) {
    let specs = &project.metadata().renders;
    let width: u16 = Field::ALL.iter().map(|field| field.width() + 1).sum();
    // Heading, a row each, and the two borders — plus one for the key hints,
    // which are the only place `ctrl-n` is discoverable.
    let height = u16::try_from(specs.len())
        .unwrap_or(u16::MAX)
        .saturating_add(5);
    let area = centred(area, width + 4, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightMagenta))
        .title(Span::styled(
            " render specifications ",
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    // Everything underneath is cleared: a modal drawn over live cells reads as
    // corruption rather than as a dialogue.
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let mut lines = vec![Line::from(
        Field::ALL
            .iter()
            .map(|field| {
                Span::styled(
                    pad(field.title(), field.width()),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect::<Vec<_>>(),
    )];

    for (row, spec) in specs.iter().enumerate() {
        let spans = Field::ALL
            .iter()
            .map(|field| {
                let here = row == editor.row && *field == editor.field;
                // The buffer, not the specification, in the cell being typed
                // into — otherwise the keystrokes would be invisible until
                // they happened to parse.
                let text = if here {
                    editor.buffer.clone()
                } else {
                    field.read(spec)
                };
                Span::styled(
                    pad(&text, field.width()),
                    if here {
                        Style::default().fg(Color::Black).bg(Color::LightMagenta)
                    } else if row == editor.row {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                )
            })
            .collect::<Vec<_>>();
        lines.push(Line::from(spans));
    }

    if specs.is_empty() {
        lines.push(Line::styled(
            "none declared — ctrl-n adds one",
            Style::default().fg(Color::DarkGray),
        ));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑↓ row   ←→ field   ctrl-n add   ctrl-d remove   esc close",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// `text` in a cell of `width`, padded or cut to fit.
fn pad(text: &str, width: u16) -> String {
    let width = usize::from(width);
    let mut cell: String = text.chars().take(width - 1).collect();
    while cell.chars().count() < width {
        cell.push(' ');
    }
    cell
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;

    fn press(editor: &mut RendersEditor, project: &mut Project, code: KeyCode) -> bool {
        editor.key(KeyEvent::new(code, KeyModifiers::NONE), project)
    }

    fn chord(editor: &mut RendersEditor, project: &mut Project, code: KeyCode) -> bool {
        editor.key(KeyEvent::new(code, KeyModifiers::CONTROL), project)
    }

    fn type_text(editor: &mut RendersEditor, project: &mut Project, text: &str) {
        for character in text.chars() {
            press(editor, project, KeyCode::Char(character));
        }
    }

    #[test]
    fn retyping_a_cell_changes_the_specification() {
        let mut project = fixtures::project();
        let mut editor = RendersEditor::new(0, &project);

        for _ in 0..editor.buffer.len() {
            press(&mut editor, &mut project, KeyCode::Backspace);
        }
        type_text(&mut editor, &mut project, "favicon-16");

        assert_eq!(project.metadata().renders[0].name, "favicon-16");
    }

    #[test]
    fn an_emptied_size_cell_does_not_become_a_zero_sized_canvas() {
        // The cell commits as it is typed, so `32` backspaced to `3` really is
        // a width of three — that is the point of editing in place. What must
        // not happen is the empty cell in the middle being taken as a zero,
        // which is a canvas the renderer refuses outright.
        let mut project = fixtures::project();
        let mut editor = RendersEditor::new(0, &project);

        // Move to the width column and clear it.
        press(&mut editor, &mut project, KeyCode::Right);
        press(&mut editor, &mut project, KeyCode::Right);
        for _ in 0..8 {
            press(&mut editor, &mut project, KeyCode::Backspace);
        }
        assert!(
            project.metadata().renders[0].width > 0,
            "an empty cell became a zero-sized canvas"
        );

        type_text(&mut editor, &mut project, "64");
        assert_eq!(project.metadata().renders[0].width, 64);
    }

    #[test]
    fn a_specification_can_be_added_and_removed() {
        let mut project = fixtures::project();
        let before = project.metadata().renders.len();
        let mut editor = RendersEditor::new(0, &project);

        assert!(chord(&mut editor, &mut project, KeyCode::Char('n')));
        assert_eq!(project.metadata().renders.len(), before + 1);
        // Added below the selection and selected, so it can be retyped at once.
        assert_eq!(editor.row(), 1);

        assert!(chord(&mut editor, &mut project, KeyCode::Char('d')));
        assert_eq!(project.metadata().renders.len(), before);
    }

    #[test]
    fn an_added_specification_is_one_the_renderer_will_accept() {
        // A row that cannot render is worse than no row: it fails the whole of
        // `shaipe render` the next time anybody runs it.
        let mut project = fixtures::project();
        let mut editor = RendersEditor::new(0, &project);
        chord(&mut editor, &mut project, KeyCode::Char('n'));

        let spec = project.metadata().renders[1].clone();
        let renderer =
            crate::render::Renderer::new(&project, crate::render::RenderOptions::default())
                .expect("the fixture renders");
        assert!(renderer.render(&spec).is_ok(), "{spec:?}");
    }

    #[test]
    fn removing_the_last_specification_leaves_the_cursor_somewhere_real() {
        // An index past the end panics on draw, which takes the terminal down
        // with it.
        let mut project = fixtures::project();
        let mut editor = RendersEditor::new(0, &project);

        while !project.metadata().renders.is_empty() {
            chord(&mut editor, &mut project, KeyCode::Char('d'));
        }

        assert_eq!(editor.row(), 0);
        assert!(!editor.closed());
    }

    #[test]
    fn escape_closes_the_editor() {
        let mut project = fixtures::project();
        let mut editor = RendersEditor::new(0, &project);

        press(&mut editor, &mut project, KeyCode::Esc);
        assert!(editor.closed());
    }

    #[test]
    fn a_modal_never_asks_for_more_room_than_the_terminal_has() {
        // Terminals get dragged to absurd sizes, and the arithmetic here is
        // subtraction on unsigned integers.
        for (width, height) in [(1, 1), (5, 3), (200, 60)] {
            let area = Rect::new(0, 0, width, height);
            let inner = centred(area, 80, 20);
            assert!(inner.width <= width && inner.height <= height);
        }
    }
}
