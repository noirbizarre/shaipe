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

use crate::project::{Background, Format, Hsl, Project, RenderSpec, Rgba, Variant};

/// What is covering the workspace.
///
/// Three variants, for the same reason: a render specification has six
/// fields, a variant has two, and a colour has three sliders — none of them
/// fit a list row. A *full* palette editor — adding, removing and renaming
/// colours — was considered once and rejected for exactly that pane-row
/// reason, and still is: the palette stays a pane, edited in place, for name
/// and role. What earns a colour a modal is narrower — the value of the row
/// already selected, with room to turn hue, saturation and lightness
/// independently. Variants and render specifications earn one for the
/// opposite reason: adding, removing and reordering them needs a table, the
/// same table the tabs above the preview already read from.
#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    /// The variants editor.
    Variants(VariantsEditor),
    /// The render specifications editor.
    Renders(RendersEditor),
    /// The colour picker.
    ColourPicker(ColourPicker),
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
    /// remove a row, and `ctrl+↑`/`ctrl+↓` move one: chords rather than `+`,
    /// `-` or plain arrows, all of which are characters or motions somebody
    /// typing a size or picking a row would otherwise reach instead.
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
            KeyCode::Up if control => return self.move_row(-1, project),
            KeyCode::Down if control => return self.move_row(1, project),
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

    /// Commit the cell, then swap the selected row with its neighbour.
    ///
    /// Does nothing at either end: there is no neighbour to swap with there,
    /// and wrapping — which the tab strip does — would be a row silently
    /// appearing to jump to the opposite end of a table instead of moving
    /// one step, which is confusing in a way it never is for tabs.
    fn move_row(&mut self, delta: isize, project: &mut Project) -> bool {
        let mutated = self.commit(project);
        let len = project.metadata().renders.len();
        let target = if delta.is_negative() {
            self.row.checked_sub(1)
        } else {
            (self.row + 1 < len).then_some(self.row + 1)
        };
        let Some(target) = target else {
            return mutated;
        };
        project.metadata_mut().renders.swap(self.row, target);
        self.row = target;
        self.load(project);
        true
    }
}

/// One column of the variants table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantField {
    /// The stable name render specifications refer to.
    Name,
    /// The id of the element in the document that draws it.
    Element,
}

impl VariantField {
    /// Every column, left to right.
    const ALL: [Self; 2] = [Self::Name, Self::Element];

    /// The column's heading.
    const fn title(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Element => "element",
        }
    }

    /// How wide it is drawn, in characters.
    const fn width(self) -> u16 {
        16
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

    /// What a variant currently says for this column.
    fn read(self, variant: &Variant) -> String {
        match self {
            Self::Name => variant.name.clone(),
            Self::Element => variant.element.clone(),
        }
    }

    /// Write `text` back, reporting whether it said anything usable.
    ///
    /// Both columns refuse an empty string: a nameless variant cannot be
    /// referred to, and an elementless one names nothing in the document.
    fn write(self, variant: &mut Variant, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        match self {
            Self::Name => variant.name = text.to_owned(),
            Self::Element => variant.element = text.to_owned(),
        }
        true
    }
}

/// The variants editor.
///
/// The same shape as [`RendersEditor`], for the same reason: a table wants
/// the width of the screen, and everything else the workspace does still
/// happens in place — the tab strip browses variants, this is what adds,
/// removes and reorders them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantsEditor {
    row: usize,
    field: VariantField,
    /// What has been typed into the current cell.
    buffer: String,
    closed: bool,
}

impl VariantsEditor {
    /// Open the editor on a row.
    #[must_use]
    pub fn new(row: usize, project: &Project) -> Self {
        let mut editor = Self {
            row,
            field: VariantField::Name,
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
            .variants
            .get(self.row)
            .map(|variant| self.field.read(variant))
            .unwrap_or_default();
    }

    /// Write the buffer back, reporting whether the project changed.
    fn commit(&self, project: &mut Project) -> bool {
        let buffer = self.buffer.clone();
        let Some(variant) = project.metadata_mut().variants.get_mut(self.row) else {
            return false;
        };
        let before = variant.clone();
        self.field.write(variant, &buffer);
        *variant != before
    }

    /// Apply a keypress, reporting whether the project changed.
    ///
    /// The key map is [`RendersEditor::key`]'s exactly: the arrows commit
    /// before moving, `ctrl-n`/`ctrl-d` add and remove a row, and
    /// `ctrl+↑`/`ctrl+↓` move one.
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
                return true;
            }
            KeyCode::Char('d') if control => return self.remove(project),
            KeyCode::Up if control => return self.move_row(-1, project),
            KeyCode::Down if control => return self.move_row(1, project),
            KeyCode::Up => {
                let mutated = self.commit(project);
                self.row = self.row.saturating_sub(1);
                self.load(project);
                return mutated;
            }
            KeyCode::Down => {
                let mutated = self.commit(project);
                let last = project.metadata().variants.len().saturating_sub(1);
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

    /// Add a variant below the one selected.
    ///
    /// Its element defaults to the selected variant's own — an alias, not a
    /// new drawing — so the row the renderer sees is one it already knows
    /// how to draw, the same guarantee [`RendersEditor::add`] gives by
    /// defaulting its `variant` field to one that already exists. The TUI
    /// cannot draw new geometry; only `write_svg` or an agent can, and this
    /// leaves the id free to be retyped onto whichever element they add.
    fn add(&mut self, project: &mut Project) {
        // Named after its position rather than left blank: an unnamed
        // variant cannot be referred to by a render specification.
        let name = format!("variant-{}", project.metadata().variants.len() + 1);
        let element = project
            .metadata()
            .variants
            .get(self.row)
            .map_or_else(|| name.clone(), |variant| variant.element.clone());

        let variants = &mut project.metadata_mut().variants;
        let at = (self.row + 1).min(variants.len());
        variants.insert(at, Variant::with_element(name, element));

        self.row = at;
        self.field = VariantField::Name;
        self.load(project);
    }

    /// Remove the selected variant.
    fn remove(&mut self, project: &mut Project) -> bool {
        let variants = &mut project.metadata_mut().variants;
        if self.row >= variants.len() {
            return false;
        }
        variants.remove(self.row);
        self.row = self.row.min(variants.len().saturating_sub(1));
        self.load(project);
        true
    }

    /// Commit the cell, then swap the selected row with its neighbour.
    ///
    /// See [`RendersEditor::move_row`] — the same rule, the same reason.
    fn move_row(&mut self, delta: isize, project: &mut Project) -> bool {
        let mutated = self.commit(project);
        let len = project.metadata().variants.len();
        let target = if delta.is_negative() {
            self.row.checked_sub(1)
        } else {
            (self.row + 1 < len).then_some(self.row + 1)
        };
        let Some(target) = target else {
            return mutated;
        };
        project.metadata_mut().variants.swap(self.row, target);
        self.row = target;
        self.load(project);
        true
    }
}

/// How far one press of an arrow key moves a slider.
///
/// Plain arrows nudge by the smallest unit each control has; `shift` jumps
/// further, the way scrubbing a real slider does. Hue moves in degrees, so
/// its step is naturally larger than a fraction's.
const HUE_STEP: f32 = 1.0;
/// `shift+←`/`shift+→` on hue.
const HUE_STEP_LARGE: f32 = 15.0;
/// A plain step on saturation or lightness, one percentage point.
const FRACTION_STEP: f32 = 0.01;
/// `shift+←`/`shift+→` on saturation or lightness, ten points.
const FRACTION_STEP_LARGE: f32 = 0.10;
/// A plain step on alpha, out of 255.
const ALPHA_STEP: i32 = 1;
/// `shift+←`/`shift+→` on alpha.
const ALPHA_STEP_LARGE: i32 = 16;

/// How wide a slider's bar is drawn, in characters.
const BAR_WIDTH: u16 = 20;

/// Which control the colour picker's keys are pointed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerField {
    /// Degrees around the colour wheel.
    Hue,
    /// Grey to fully saturated.
    Saturation,
    /// Black to white.
    Lightness,
    /// Transparent to opaque.
    Alpha,
    /// The raw hex text, typed rather than stepped.
    Hex,
}

impl PickerField {
    /// Every control, in the order the picker lists them.
    const ALL: [Self; 5] = [
        Self::Hue,
        Self::Saturation,
        Self::Lightness,
        Self::Alpha,
        Self::Hex,
    ];

    /// The control's label.
    const fn title(self) -> &'static str {
        match self {
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Lightness => "lightness",
            Self::Alpha => "alpha",
            Self::Hex => "hex",
        }
    }

    /// Its position, for moving between controls.
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or_default()
    }

    /// The next control down.
    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    /// The previous control up.
    fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// The colour picker.
///
/// Works in HSL rather than the project's own `Rgba`: hue, saturation and
/// lightness are the three knobs an eye actually turns, and stepping raw
/// red/green/blue moves a colour sideways more often than it moves it where
/// intended. `Rgba` — with its own alpha, which HSL says nothing about — is
/// only where the result is stored, in [`Palette::colour_mut`] and, if the
/// artwork binds to this name, in the document itself via
/// [`Project::restyle`].
///
/// [`Palette::colour_mut`]: crate::project::Palette::colour_mut
#[derive(Debug, Clone, PartialEq)]
pub struct ColourPicker {
    row: usize,
    hsl: Hsl,
    alpha: u8,
    field: PickerField,
    /// What has been typed into the hex field.
    ///
    /// Only meaningful while `field` is [`PickerField::Hex`] — the sliders
    /// have no text of their own to buffer, only a number to step, and step
    /// straight into `hsl`/`alpha` instead.
    hex_buffer: String,
    closed: bool,
}

impl ColourPicker {
    /// Open the picker on a palette row.
    #[must_use]
    pub fn new(row: usize, project: &Project) -> Self {
        let value = project
            .metadata()
            .palette
            .colours()
            .get(row)
            .map_or(Rgba::new(0, 0, 0, u8::MAX), |colour| colour.value);
        Self {
            row,
            hsl: Hsl::from(value),
            alpha: value.a,
            field: PickerField::Hue,
            hex_buffer: value.to_string(),
            closed: false,
        }
    }

    /// Which row is being edited.
    #[must_use]
    pub const fn row(&self) -> usize {
        self.row
    }

    /// Whether the picker has been asked to close.
    #[must_use]
    pub const fn closed(&self) -> bool {
        self.closed
    }

    /// The colour the sliders currently describe.
    fn value(&self) -> Rgba {
        let (r, g, b) = self.hsl.to_rgb();
        Rgba::new(r, g, b, self.alpha)
    }

    /// Write a colour to the palette row, restyling bound artwork if it
    /// changed, and reporting whether it did.
    ///
    /// Takes the colour explicitly rather than always reading [`Self::value`]:
    /// a hex value just typed is written exactly as parsed, not rounded
    /// through [`Hsl`] and back first, which is what would otherwise make the
    /// last character of a typed hex code look like it never quite took.
    fn commit(&mut self, value: Rgba, project: &mut Project) -> bool {
        // Read before anything is mutated, for the same reason
        // `commit_palette_field` does: a value change restyles whatever the
        // artwork already binds under the name the colour has *now*.
        let Some(name) = project
            .metadata()
            .palette
            .colours()
            .get(self.row)
            .map(|colour| colour.name.clone())
        else {
            return false;
        };

        let changed = {
            let Some(colour) = project.metadata_mut().palette.colour_mut(self.row) else {
                return false;
            };
            if colour.value == value {
                false
            } else {
                colour.value = value;
                true
            }
        };

        if changed {
            project
                .restyle(&name, value)
                .expect("the project's own document already parsed; only bound attributes change");
        }
        // The hex buffer is deliberately left alone here: it only needs to
        // agree with the stored value when the hex field is entered (see
        // `enter_field`), and rewriting it after every slider step or every
        // partial parse while a hex code is still being typed would stomp on
        // keystrokes the buffer has not seen yet — the same trap
        // `commit_palette_field` avoids by never touching its own buffer.
        changed
    }

    /// Step the field currently selected, reporting whether the project
    /// changed.
    ///
    /// A no-op on [`PickerField::Hex`], which the caller already excludes —
    /// arrows step a number, and hex is typed, not stepped.
    fn step(&mut self, positive: bool, big: bool, project: &mut Project) -> bool {
        let sign = if positive { 1.0 } else { -1.0 };
        match self.field {
            PickerField::Hue => {
                let step = if big { HUE_STEP_LARGE } else { HUE_STEP };
                // Wraps rather than clamps: hue is a wheel, and 359° plus one
                // more step is 0°, not stuck at the edge.
                self.hsl.h = (self.hsl.h + sign * step).rem_euclid(360.0);
            }
            PickerField::Saturation => {
                let step = if big {
                    FRACTION_STEP_LARGE
                } else {
                    FRACTION_STEP
                };
                self.hsl.s = (self.hsl.s + sign * step).clamp(0.0, 1.0);
            }
            PickerField::Lightness => {
                let step = if big {
                    FRACTION_STEP_LARGE
                } else {
                    FRACTION_STEP
                };
                self.hsl.l = (self.hsl.l + sign * step).clamp(0.0, 1.0);
            }
            PickerField::Alpha => {
                let step = if big { ALPHA_STEP_LARGE } else { ALPHA_STEP };
                let delta = if positive { step } else { -step };
                let stepped = i32::from(self.alpha) + delta;
                self.alpha = stepped.clamp(0, i32::from(u8::MAX)) as u8;
            }
            PickerField::Hex => return false,
        }
        let value = self.value();
        self.commit(value, project)
    }

    /// Commit the hex buffer on leaving the hex field, reporting whether the
    /// project changed.
    ///
    /// Every other field already committed itself on every keystroke; the
    /// buffer only exists because typing is not stepping.
    fn leave_field(&mut self, project: &mut Project) -> bool {
        if self.field == PickerField::Hex {
            self.try_apply_hex(project)
        } else {
            false
        }
    }

    /// Reload state for the field just entered.
    ///
    /// Only [`PickerField::Hex`] needs it: the sliders read straight from
    /// `hsl`/`alpha`, always current, but the hex buffer is free text and
    /// would otherwise still say whatever was last typed towards some
    /// earlier value.
    fn enter_field(&mut self) {
        if self.field == PickerField::Hex {
            self.hex_buffer = self.value().to_string();
        }
    }

    /// Parse the hex buffer and write it to the project if it parses.
    ///
    /// An unparsable buffer is kept and shown, never written — `#f0` is what
    /// every colour looks like halfway through being typed, and refusing the
    /// keystroke would make the field impossible to use. Mirrors
    /// `commit_palette_field`'s rule for the pane's own hex field exactly.
    fn try_apply_hex(&mut self, project: &mut Project) -> bool {
        let Ok(parsed) = self.hex_buffer.trim().parse::<Rgba>() else {
            return false;
        };
        self.hsl = Hsl::from(parsed);
        self.alpha = parsed.a;
        self.commit(parsed, project)
    }

    /// Apply a keypress, reporting whether the project changed.
    ///
    /// Up and down move between the picker's five controls — four sliders and
    /// the hex field — and commit on the way, the same "leaving a field is as
    /// good as finishing it" rule `RendersEditor` and the palette's own
    /// in-place editor already follow. Left and right step whichever slider
    /// is selected; backspace and typing belong to the hex field alone, the
    /// only one with text to hold.
    pub fn key(&mut self, key: KeyEvent, project: &mut Project) -> bool {
        let big = key.modifiers.contains(KeyModifiers::SHIFT);

        match key.code {
            // Nothing left to write here: every valid change already reached
            // the project as it was made, in `commit`.
            KeyCode::Esc | KeyCode::Enter => {
                self.closed = true;
                false
            }
            KeyCode::Down | KeyCode::Tab => {
                let changed = self.leave_field(project);
                self.field = self.field.next();
                self.enter_field();
                changed
            }
            KeyCode::Up | KeyCode::BackTab => {
                let changed = self.leave_field(project);
                self.field = self.field.previous();
                self.enter_field();
                changed
            }
            KeyCode::Left if self.field != PickerField::Hex => self.step(false, big, project),
            KeyCode::Right if self.field != PickerField::Hex => self.step(true, big, project),
            KeyCode::Backspace if self.field == PickerField::Hex => {
                self.hex_buffer.pop();
                self.try_apply_hex(project)
            }
            KeyCode::Char(character) if self.field == PickerField::Hex => {
                self.hex_buffer.push(character);
                self.try_apply_hex(project)
            }
            _ => false,
        }
    }
}

/// A slider's bar, `width` characters wide and filled in proportion to
/// `fraction`.
fn bar(fraction: f32, width: u16) -> String {
    let width = usize::from(width);
    let filled = ((fraction.clamp(0.0, 1.0) * width as f32).round() as usize).min(width);
    let mut drawn = String::with_capacity(width);
    for position in 0..width {
        drawn.push(if position < filled { '█' } else { '·' });
    }
    drawn
}

/// Draw the colour picker over the workspace.
pub fn draw_colour_picker(
    frame: &mut Frame<'_>,
    picker: &ColourPicker,
    project: &Project,
    area: Rect,
) {
    let name = project
        .metadata()
        .palette
        .colours()
        .get(picker.row)
        .map_or("", |colour| colour.name.as_str());

    let width = BAR_WIDTH + 22;
    // One row per control, the swatch, a blank line, the key hints, and the
    // two borders.
    let height = u16::try_from(PickerField::ALL.len())
        .unwrap_or(5)
        .saturating_add(7);
    let area = centred(area, width, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightCyan))
        .title(Span::styled(
            format!(" {name} "),
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    // Everything underneath is cleared: a modal drawn over live cells reads
    // as corruption rather than as a dialogue.
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let value = picker.value();
    let [swatch_area, rest] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .areas(inner);
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(value.r, value.g, value.b))),
        swatch_area,
    );

    let mut lines = Vec::new();
    for field in PickerField::ALL {
        let selected = field == picker.field;
        let label_style = if selected {
            Style::default().fg(Color::Black).bg(Color::LightCyan)
        } else {
            Style::default().fg(Color::Gray)
        };
        let mut spans = vec![Span::styled(pad(field.title(), 12), label_style)];

        if field == PickerField::Hex {
            // Shown, not refused, exactly as the palette pane's own hex field
            // behaves: it is on its way to being a colour, and nothing has
            // reached the document until it parses.
            let valid = picker.hex_buffer.trim().parse::<Rgba>().is_ok();
            let text_style = match (selected, valid) {
                (false, _) => Style::default().fg(Color::DarkGray),
                (true, true) => Style::default().fg(Color::Black).bg(Color::LightCyan),
                (true, false) => Style::default().fg(Color::Black).bg(Color::LightRed),
            };
            spans.push(Span::styled(picker.hex_buffer.clone(), text_style));
        } else {
            let (fraction, reading) = match field {
                PickerField::Hue => (picker.hsl.h / 360.0, format!("{:>3.0}°", picker.hsl.h)),
                PickerField::Saturation => {
                    (picker.hsl.s, format!("{:>3.0}%", picker.hsl.s * 100.0))
                }
                PickerField::Lightness => (picker.hsl.l, format!("{:>3.0}%", picker.hsl.l * 100.0)),
                PickerField::Alpha => (
                    f32::from(picker.alpha) / 255.0,
                    format!("{:>3}", picker.alpha),
                ),
                PickerField::Hex => unreachable!("handled above"),
            };
            spans.push(Span::styled(
                bar(fraction, BAR_WIDTH),
                Style::default().fg(Color::LightCyan),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::raw(reading));
        }

        lines.push(Line::from(spans));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑↓ field   ←→ adjust   shift ×10   type hex   esc close",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(lines), rest);
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
pub fn draw_renders(frame: &mut Frame<'_>, editor: &RendersEditor, project: &Project, area: Rect) {
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
        "↑↓ row   ←→ field   ctrl-n add   ctrl-d remove   ctrl+↑↓ move   esc close",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draw the variants editor over the workspace.
pub fn draw_variants(
    frame: &mut Frame<'_>,
    editor: &VariantsEditor,
    project: &Project,
    area: Rect,
) {
    let variants = &project.metadata().variants;
    let width: u16 = VariantField::ALL
        .iter()
        .map(|field| field.width() + 1)
        .sum();
    // Heading, a row each, and the two borders — plus one for the key hints,
    // which are the only place `ctrl-n` is discoverable.
    let height = u16::try_from(variants.len())
        .unwrap_or(u16::MAX)
        .saturating_add(5);
    let area = centred(area, width + 4, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightGreen))
        .title(Span::styled(
            " variants ",
            Style::default()
                .fg(Color::LightGreen)
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
        VariantField::ALL
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

    for (row, variant) in variants.iter().enumerate() {
        let spans = VariantField::ALL
            .iter()
            .map(|field| {
                let here = row == editor.row && *field == editor.field;
                // The buffer, not the variant, in the cell being typed into —
                // otherwise the keystrokes would be invisible until they
                // happened to parse.
                let text = if here {
                    editor.buffer.clone()
                } else {
                    field.read(variant)
                };
                Span::styled(
                    pad(&text, field.width()),
                    if here {
                        Style::default().fg(Color::Black).bg(Color::LightGreen)
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

    if variants.is_empty() {
        lines.push(Line::styled(
            "none declared — ctrl-n adds one",
            Style::default().fg(Color::DarkGray),
        ));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑↓ row   ←→ field   ctrl-n add   ctrl-d remove   ctrl+↑↓ move   esc close",
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
    fn a_specification_can_be_moved_up_and_down() {
        let mut project = fixtures::project();
        let mut editor = RendersEditor::new(0, &project);
        let first = project.metadata().renders[0].name.clone();
        let second = project.metadata().renders[1].name.clone();

        assert!(chord(&mut editor, &mut project, KeyCode::Down));
        assert_eq!(editor.row(), 1);
        assert_eq!(project.metadata().renders[0].name, second);
        assert_eq!(project.metadata().renders[1].name, first);

        assert!(chord(&mut editor, &mut project, KeyCode::Up));
        assert_eq!(editor.row(), 0);
        assert_eq!(project.metadata().renders[0].name, first);
        assert_eq!(project.metadata().renders[1].name, second);
    }

    #[test]
    fn moving_the_top_row_up_or_the_bottom_row_down_does_nothing() {
        // Wrapping, the way the tab strip does, would make a row appear to
        // jump to the opposite end of the table instead of moving one step.
        let mut project = fixtures::project();
        let before = project.metadata().renders.clone();

        let mut top = RendersEditor::new(0, &project);
        assert!(!chord(&mut top, &mut project, KeyCode::Up));
        assert_eq!(project.metadata().renders, before);

        let last = before.len() - 1;
        let mut bottom = RendersEditor::new(last, &project);
        assert!(!chord(&mut bottom, &mut project, KeyCode::Down));
        assert_eq!(project.metadata().renders, before);
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

    fn press_variant(editor: &mut VariantsEditor, project: &mut Project, code: KeyCode) -> bool {
        editor.key(KeyEvent::new(code, KeyModifiers::NONE), project)
    }

    fn chord_variant(editor: &mut VariantsEditor, project: &mut Project, code: KeyCode) -> bool {
        editor.key(KeyEvent::new(code, KeyModifiers::CONTROL), project)
    }

    fn type_text_variant(editor: &mut VariantsEditor, project: &mut Project, text: &str) {
        for character in text.chars() {
            press_variant(editor, project, KeyCode::Char(character));
        }
    }

    #[test]
    fn retyping_a_variant_cell_changes_it() {
        let mut project = fixtures::project();
        let mut editor = VariantsEditor::new(0, &project);

        for _ in 0..editor.buffer.len() {
            press_variant(&mut editor, &mut project, KeyCode::Backspace);
        }
        type_text_variant(&mut editor, &mut project, "mark");

        assert_eq!(project.metadata().variants[0].name, "mark");
    }

    #[test]
    fn an_emptied_variant_name_is_not_committed_as_empty() {
        // The cell commits as it is typed, so "icon" backspaced down to "i"
        // really is a variant named "i" — that is the point of editing in
        // place. What must not happen is the last backspace, to nothing at
        // all, reaching the project: a nameless variant cannot be referred
        // to by any render specification.
        let mut project = fixtures::project();
        let mut editor = VariantsEditor::new(0, &project);

        for _ in 0..editor.buffer.len() {
            press_variant(&mut editor, &mut project, KeyCode::Backspace);
        }

        assert_eq!(project.metadata().variants[0].name, "i");
    }

    #[test]
    fn a_variant_can_be_added_and_removed() {
        let mut project = fixtures::project();
        let before = project.metadata().variants.len();
        let mut editor = VariantsEditor::new(0, &project);

        assert!(chord_variant(&mut editor, &mut project, KeyCode::Char('n')));
        assert_eq!(project.metadata().variants.len(), before + 1);
        // Added below the selection and selected, so it can be retyped at once.
        assert_eq!(editor.row(), 1);
        // Aliases the element of the row it was added from, so the renderer
        // already knows how to draw it.
        assert_eq!(
            project.metadata().variants[1].element,
            project.metadata().variants[0].element
        );

        assert!(chord_variant(&mut editor, &mut project, KeyCode::Char('d')));
        assert_eq!(project.metadata().variants.len(), before);
    }

    #[test]
    fn an_added_variant_is_one_the_renderer_will_accept() {
        // A row that cannot render is worse than no row: it fails as soon as
        // anything asks for its preview.
        let mut project = fixtures::project();
        let mut editor = VariantsEditor::new(0, &project);
        chord_variant(&mut editor, &mut project, KeyCode::Char('n'));

        let variant = project.metadata().variants[1].clone();
        let renderer =
            crate::render::Renderer::new(&project, crate::render::RenderOptions::default())
                .expect("the fixture renders");
        let spec = RenderSpec::square(&variant.name, &variant.name, 64);
        assert!(renderer.render(&spec).is_ok(), "{spec:?}");
    }

    #[test]
    fn a_variant_can_be_moved_up_and_down() {
        let mut project = fixtures::project();
        let mut editor = VariantsEditor::new(0, &project);
        let first = project.metadata().variants[0].name.clone();
        let second = project.metadata().variants[1].name.clone();

        assert!(chord_variant(&mut editor, &mut project, KeyCode::Down));
        assert_eq!(editor.row(), 1);
        assert_eq!(project.metadata().variants[0].name, second);
        assert_eq!(project.metadata().variants[1].name, first);

        assert!(chord_variant(&mut editor, &mut project, KeyCode::Up));
        assert_eq!(editor.row(), 0);
        assert_eq!(project.metadata().variants[0].name, first);
        assert_eq!(project.metadata().variants[1].name, second);
    }

    #[test]
    fn removing_the_last_variant_leaves_the_cursor_somewhere_real() {
        // An index past the end panics on draw, which takes the terminal
        // down with it.
        let mut project = fixtures::project();
        let mut editor = VariantsEditor::new(0, &project);

        while !project.metadata().variants.is_empty() {
            chord_variant(&mut editor, &mut project, KeyCode::Char('d'));
        }

        assert_eq!(editor.row(), 0);
        assert!(!editor.closed());
    }

    #[test]
    fn escape_closes_the_variants_editor() {
        let mut project = fixtures::project();
        let mut editor = VariantsEditor::new(0, &project);

        press_variant(&mut editor, &mut project, KeyCode::Esc);
        assert!(editor.closed());
    }

    fn press_picker(picker: &mut ColourPicker, project: &mut Project, code: KeyCode) -> bool {
        picker.key(KeyEvent::new(code, KeyModifiers::NONE), project)
    }

    fn press_picker_shift(picker: &mut ColourPicker, project: &mut Project, code: KeyCode) -> bool {
        picker.key(KeyEvent::new(code, KeyModifiers::SHIFT), project)
    }

    fn type_hex(picker: &mut ColourPicker, project: &mut Project, text: &str) {
        for character in text.chars() {
            press_picker(picker, project, KeyCode::Char(character));
        }
    }

    /// Move the picker onto the hex field, from the hue field it opens on.
    fn go_to_hex(picker: &mut ColourPicker, project: &mut Project) {
        for _ in 0..PickerField::ALL.len() - 1 {
            press_picker(picker, project, KeyCode::Down);
        }
    }

    #[test]
    fn the_right_arrow_steps_the_selected_slider() {
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        let before = picker.hsl.h;

        assert!(press_picker(&mut picker, &mut project, KeyCode::Right));

        assert_eq!(picker.hsl.h, (before + HUE_STEP).rem_euclid(360.0));
        assert_eq!(
            project.metadata().palette.colours()[0].value,
            picker.value()
        );
    }

    #[test]
    fn shift_takes_a_bigger_step_than_a_plain_arrow() {
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        let before = picker.hsl.h;

        press_picker_shift(&mut picker, &mut project, KeyCode::Right);

        assert_eq!(picker.hsl.h, (before + HUE_STEP_LARGE).rem_euclid(360.0));
    }

    #[test]
    fn hue_wraps_at_the_ends_of_the_wheel_instead_of_clamping() {
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        picker.hsl.h = 359.0;

        press_picker(&mut picker, &mut project, KeyCode::Right);

        assert_eq!(picker.hsl.h, 0.0);
    }

    #[test]
    fn arrows_do_nothing_on_the_hex_field() {
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        go_to_hex(&mut picker, &mut project);
        assert_eq!(picker.field, PickerField::Hex);

        let before = project.metadata().palette.colours()[0].value;
        assert!(!press_picker(&mut picker, &mut project, KeyCode::Right));
        assert_eq!(project.metadata().palette.colours()[0].value, before);
    }

    #[test]
    fn typing_a_full_hex_value_commits_it_and_updates_the_sliders() {
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        go_to_hex(&mut picker, &mut project);
        for _ in 0..picker.hex_buffer.len() {
            press_picker(&mut picker, &mut project, KeyCode::Backspace);
        }
        type_hex(&mut picker, &mut project, "#0066ff");

        assert_eq!(
            project.metadata().palette.colours()[0].value,
            Rgba::new(0x00, 0x66, 0xff, 0xff)
        );
        // The sliders agree with what was typed, not just the stored value.
        assert!((picker.hsl.h - 216.0).abs() < 1.0, "{}", picker.hsl.h);
    }

    #[test]
    fn an_unparsable_hex_value_is_kept_and_shown_but_never_committed() {
        // `#f0` is what every hex colour looks like halfway through being
        // typed. Refusing the keystroke would make the field impossible to
        // use, so it is shown and simply never reaches the project. Typed in
        // deliberately rather than reached by backspacing a real colour down:
        // some of *that* colour's own prefixes are themselves valid short
        // forms, which would commit on the way and defeat the point of the
        // test.
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        go_to_hex(&mut picker, &mut project);
        while !picker.hex_buffer.is_empty() {
            press_picker(&mut picker, &mut project, KeyCode::Backspace);
        }
        let before = project.metadata().palette.colours()[0].value;

        type_hex(&mut picker, &mut project, "#f0");

        assert_eq!(picker.hex_buffer, "#f0");
        assert_eq!(project.metadata().palette.colours()[0].value, before);
    }

    #[test]
    fn leaving_an_invalid_hex_value_discards_it_without_writing_anything() {
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        go_to_hex(&mut picker, &mut project);
        while !picker.hex_buffer.is_empty() {
            press_picker(&mut picker, &mut project, KeyCode::Backspace);
        }
        let before = project.metadata().palette.colours()[0].value;

        type_hex(&mut picker, &mut project, "#f0");
        assert_eq!(picker.hex_buffer, "#f0");

        // Leave, then come back: the half-typed text is gone, replaced by
        // whatever the colour still is.
        press_picker(&mut picker, &mut project, KeyCode::Up);
        press_picker(&mut picker, &mut project, KeyCode::Down);

        assert_eq!(picker.hex_buffer, before.to_string());
        assert_eq!(project.metadata().palette.colours()[0].value, before);
    }

    #[test]
    fn adjusting_the_colour_restyles_bound_artwork() {
        // The same proof `editing_a_palette_colours_value_in_the_tui_restyles_bound_artwork`
        // gives the in-place editor: an element bound to the colour by name
        // is restyled the moment the picker writes a new value, not only on
        // close.
        const BOUND: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 64 64">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:palette>
        <shaipe:color name="accent" value="#f05032" role="accent"/>
      </shaipe:palette>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
      </shaipe:variants>
    </shaipe:project>
  </metadata>
  <symbol id="icon" viewBox="0 0 64 64"><rect width="64" height="64" fill="#f05032" shaipe:fill="accent"/></symbol>
  <use href="#icon" width="64" height="64"/>
</svg>
"##;
        let mut project = Project::from_source("logo.svg", BOUND.to_owned()).unwrap();
        let mut picker = ColourPicker::new(0, &project);
        go_to_hex(&mut picker, &mut project);
        for _ in 0..picker.hex_buffer.len() {
            press_picker(&mut picker, &mut project, KeyCode::Backspace);
        }
        type_hex(&mut picker, &mut project, "#0066ff");

        assert!(project.source().contains(r##"fill="#0066ff""##));
        assert!(!project.source().contains(r##"fill="#f05032""##));
    }

    #[test]
    fn escape_and_enter_both_close_the_picker() {
        let mut project = fixtures::project();

        let mut picker = ColourPicker::new(0, &project);
        press_picker(&mut picker, &mut project, KeyCode::Esc);
        assert!(picker.closed());

        let mut picker = ColourPicker::new(0, &project);
        press_picker(&mut picker, &mut project, KeyCode::Enter);
        assert!(picker.closed());
    }

    #[test]
    fn up_and_down_cycle_through_every_control_and_wrap() {
        let mut project = fixtures::project();
        let mut picker = ColourPicker::new(0, &project);
        assert_eq!(picker.field, PickerField::Hue);

        for expected in [
            PickerField::Saturation,
            PickerField::Lightness,
            PickerField::Alpha,
            PickerField::Hex,
            PickerField::Hue,
        ] {
            press_picker(&mut picker, &mut project, KeyCode::Down);
            assert_eq!(picker.field, expected);
        }

        press_picker(&mut picker, &mut project, KeyCode::Up);
        assert_eq!(picker.field, PickerField::Hex);
    }
}
