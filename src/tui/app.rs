//! The workspace's state.
//!
//! Deliberately plain: fields and small methods, no framework. The preview is
//! the only derived state, and it is cached because rendering on every frame
//! would rasterise a 512-pixel SVG four times a second for no reason.
//!
//! This is where editing will land. A prompt editor, a colour picker and an
//! agent conversation all need somewhere to hold a draft and a dirty flag, and
//! that somewhere is here rather than spread through the drawing code.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::preview::Image;
use crate::project::{Format, Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};
use crate::tui::render_worker::{Rendered, Worker};

/// The narrowest the description column may be dragged.
///
/// Below this the hex colours and render sizes no longer fit, and the column
/// stops being readable rather than merely cramped.
pub const MIN_COLUMN: u16 = 24;

/// How much room the preview column always keeps.
pub const MIN_PREVIEW: u16 = 12;

/// The description column's width before anyone drags it.
pub const DEFAULT_COLUMN: u16 = 38;

/// How close together two clicks count as one double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The largest a preview is rasterised, before it is fitted to the pane.
const PREVIEW_SIZE: u32 = 512;

/// The smallest a preview is allowed to shrink to.
///
/// Below this a logo stops being recognisable, and the rasterising is cheap
/// enough that shrinking further buys nothing.
const MIN_PREVIEW_SIZE: u32 = 32;

/// How long the selection must sit still before a render starts.
///
/// Long enough that arrowing through a list renders once at the end rather
/// than once per entry; short enough that a single deliberate step still feels
/// immediate.
const DEBOUNCE: Duration = Duration::from_millis(120);

/// Where a double-clicked render specification is written.
///
/// The conventional output directory, matching `shaipe render`'s default, so
/// the workspace and the command line do not disagree about where assets go.
const EXPORT_DIRECTORY: &str = "dist";

/// Which pane the keyboard is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The prompt.
    Prompt,
    /// The palette.
    Palette,
    /// The variants.
    Variants,
    /// The render specifications.
    Renders,
}

impl Focus {
    /// Every pane, in the order `Tab` visits them.
    pub const ALL: [Self; 4] = [Self::Prompt, Self::Palette, Self::Variants, Self::Renders];

    /// How many panes there are.
    pub const COUNT: usize = Self::ALL.len();

    /// The pane's title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Palette => "palette",
            Self::Variants => "variants",
            Self::Renders => "renders",
        }
    }
}

/// What the preview pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    /// Not rendered yet.
    Pending,
    /// A rendered image, and what it was rendered from.
    Ready {
        /// The pixels.
        image: Box<Image>,
        /// A description of what produced them.
        caption: String,
    },
    /// Rendering failed. Shown in the pane rather than taking the workspace
    /// down: a project being edited towards correctness spends time broken,
    /// and quitting on every intermediate state would make it unusable.
    Failed(String),
}

/// The workspace.
pub struct App {
    /// The project being worked on.
    pub project: Project,
    /// Which pane has the keyboard.
    pub focus: Focus,
    /// Whether the event loop should stop.
    pub should_quit: bool,
    /// The name of the preview backend in use, shown in the status line so a
    /// preview that looks wrong is diagnosable without a debugger.
    pub backend: &'static str,
    /// How much detail to show. Only the render timing depends on it.
    pub verbose: u8,

    /// How wide the description column is. Draggable.
    pub column_width: u16,
    /// A transient message, shown in the status line until something replaces
    /// it. Used to report what an export actually wrote.
    pub notice: Option<String>,

    /// Where each pane was drawn last frame.
    ///
    /// Recorded during drawing rather than recomputed when a click arrives:
    /// two copies of the layout arithmetic would drift, and the bug that
    /// caused would be a click landing on the wrong pane.
    areas: [Rect; Focus::COUNT],
    preview_area: Rect,
    /// The preview pane's size in pixels, or `(0, 0)` before the first frame.
    preview_pixels: (u32, u32),
    /// The last click, for recognising a double-click.
    last_click: Option<(Focus, u16, Instant)>,
    /// Whether the column divider is being dragged.
    resizing: bool,
    /// Scroll state per list pane, so the selection stays visible when a pane
    /// is smaller than its contents.
    list_states: [ListState; Focus::COUNT],

    selected: [usize; Focus::COUNT],
    preview: Preview,
    /// What the on-screen preview was rendered from.
    rendered_from: Option<RenderSpec>,
    /// What the selection currently calls for.
    desired: Option<RenderSpec>,
    /// When `desired` last changed, for the debounce.
    changed_at: Instant,
    /// The request the worker is busy with, and when it was sent.
    in_flight: Option<(u64, RenderSpec, Instant)>,
    /// The last request number handed out.
    sequence: u64,
    /// How long the last completed render took.
    last_render: Option<Duration>,
    /// Renders previews without blocking the drawing thread.
    worker: Worker,
    /// Bumped whenever the preview image is replaced.
    ///
    /// The preview backend keeps the image in encoded form and must not be
    /// asked to compare pixels to notice a change; a counter is both cheaper
    /// and impossible to get subtly wrong.
    generation: u64,
}

impl App {
    /// Open a project in the workspace.
    #[must_use]
    pub fn new(project: Project, backend: &'static str) -> Self {
        let project_for_worker = project.clone();
        Self {
            project,
            focus: Focus::Variants,
            should_quit: false,
            backend,
            verbose: 0,
            column_width: DEFAULT_COLUMN,
            notice: None,
            areas: [Rect::ZERO; Focus::COUNT],
            preview_area: Rect::ZERO,
            preview_pixels: (0, 0),
            last_click: None,
            resizing: false,
            list_states: std::array::from_fn(|_| ListState::default()),
            selected: [0; Focus::COUNT],
            preview: Preview::Pending,
            rendered_from: None,
            desired: None,
            // In the past, so the first preview starts at once rather than
            // making the workspace wait out a debounce it has no reason to.
            changed_at: Instant::now() - DEBOUNCE,
            in_flight: None,
            sequence: 0,
            last_render: None,
            worker: Worker::new(&project_for_worker),
            generation: 0,
        }
    }

    /// The index of the focused pane, for indexing `selected`.
    fn focus_index(&self) -> usize {
        Self::index_of(self.focus)
    }

    /// A pane's position in the fixed pane order.
    fn index_of(focus: Focus) -> usize {
        Focus::ALL
            .iter()
            .position(|candidate| *candidate == focus)
            .unwrap_or_default()
    }

    /// How many selectable rows the focused pane has.
    fn focused_len(&self) -> usize {
        let metadata = self.project.metadata();
        match self.focus {
            Focus::Prompt => 0,
            Focus::Palette => metadata.palette.len(),
            Focus::Variants => metadata.variants.len(),
            Focus::Renders => metadata.renders.len(),
        }
    }

    /// Move the keyboard to the next pane.
    pub fn focus_next(&mut self) {
        self.focus = Focus::ALL[(self.focus_index() + 1) % Focus::COUNT];
    }

    /// Move the keyboard to the previous pane.
    pub fn focus_previous(&mut self) {
        self.focus = Focus::ALL[(self.focus_index() + Focus::COUNT - 1) % Focus::COUNT];
    }

    /// Move the selection down within the focused pane.
    pub fn select_next(&mut self) {
        let len = self.focused_len();
        if len == 0 {
            return;
        }
        let index = self.focus_index();
        // Wrapping, because a four-item list is faster to cycle than to
        // realise you have hit the end of.
        self.selected[index] = (self.selected[index] + 1) % len;
    }

    /// Move the selection up within the focused pane.
    pub fn select_previous(&mut self) {
        let len = self.focused_len();
        if len == 0 {
            return;
        }
        let index = self.focus_index();
        self.selected[index] = (self.selected[index] + len - 1) % len;
    }

    /// The selected row in a given pane.
    #[must_use]
    pub fn selection(&self, focus: Focus) -> usize {
        let index = Focus::ALL
            .iter()
            .position(|candidate| *candidate == focus)
            .unwrap_or_default();
        // Clamped rather than trusted: metadata can shrink under a selection
        // once editing exists, and an out-of-range index would panic on draw.
        self.selected[index].min(self.pane_len(focus).saturating_sub(1))
    }

    /// How many rows a given pane has.
    fn pane_len(&self, focus: Focus) -> usize {
        let metadata = self.project.metadata();
        match focus {
            Focus::Prompt => 0,
            Focus::Palette => metadata.palette.len(),
            Focus::Variants => metadata.variants.len(),
            Focus::Renders => metadata.renders.len(),
        }
    }

    /// The selected variant's index.
    #[must_use]
    pub fn selected_variant(&self) -> usize {
        self.selection(Focus::Variants)
    }

    /// What the preview should be showing.
    ///
    /// A selected render specification wins, because it is the more specific
    /// statement: it names a variant *and* a size and background. Otherwise
    /// the selected variant is previewed at a default size.
    #[must_use]
    pub fn preview_spec(&self) -> Option<RenderSpec> {
        let metadata = self.project.metadata();

        let mut spec = if self.focus == Focus::Renders
            && let Some(spec) = metadata.renders.get(self.selection(Focus::Renders))
        {
            let mut spec = spec.clone();
            // A preview is pixels on a screen, so an SVG specification is
            // previewed by rasterising it. Handing SVG bytes to the preview
            // layer instead put the words "not a PNG" in the pane, which is
            // true, useless, and looks like a bug in the project.
            spec.format = Format::Png;
            spec
        } else {
            let variant = metadata.variants.get(self.selected_variant())?;
            RenderSpec::square(&variant.name, &variant.name, PREVIEW_SIZE)
        };

        self.fit_to_pane(&mut spec);
        Some(spec)
    }

    /// Shrink a specification to something the pane can actually show.
    ///
    /// Previewing `social` at its declared 1280x640 rasterises 819 000 pixels
    /// to be drawn in a pane holding a few thousand — which is where the
    /// workspace's stutter came from.
    ///
    /// Halving, rather than scaling to fit exactly, for three reasons: the
    /// aspect ratio stays exact, so nothing letterboxes that would not have;
    /// the size changes in a handful of steps, so dragging the divider does
    /// not re-render on every mouse event; and a power-of-two reduction is the
    /// one resamplers are kindest to.
    ///
    /// Never enlarges. A preview must not invent detail the asset lacks.
    fn fit_to_pane(&self, spec: &mut RenderSpec) {
        let (box_width, box_height) = self.preview_pixels;
        if box_width == 0 || box_height == 0 {
            return;
        }

        while spec.width > box_width || spec.height > box_height {
            let (half_width, half_height) = (spec.width / 2, spec.height / 2);
            if half_width < MIN_PREVIEW_SIZE || half_height < MIN_PREVIEW_SIZE {
                break;
            }
            spec.width = half_width;
            spec.height = half_height;
        }
    }

    /// Throw the current preview away and render it again.
    ///
    /// Also hands the worker the project as it stands, so this is what picks
    /// up a change to the file rather than merely redrawing what was already
    /// rendered.
    pub fn invalidate_preview(&mut self) {
        self.worker.reload(&self.project);
        self.rendered_from = None;
        self.desired = None;
        self.in_flight = None;
    }

    /// Whether a render is in progress.
    ///
    /// Drives the spinner, and the shorter poll interval that lets it animate.
    #[must_use]
    pub const fn is_rendering(&self) -> bool {
        self.in_flight.is_some()
    }

    /// How long the last completed render took.
    #[must_use]
    pub const fn last_render(&self) -> Option<Duration> {
        self.last_render
    }

    /// The spinner's current frame, or `None` when nothing is rendering.
    ///
    /// Derived from how long the render has been running rather than from a
    /// counter incremented per frame, so it turns at a steady rate however
    /// often the workspace happens to redraw.
    #[must_use]
    pub fn spinner(&self) -> Option<&'static str> {
        const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        const PERIOD: Duration = Duration::from_millis(90);

        let (_, _, started) = self.in_flight.as_ref()?;
        let step = started.elapsed().as_millis() / PERIOD.as_millis();
        Some(FRAMES[(step as usize) % FRAMES.len()])
    }

    /// Start a render if the selection has settled on something new.
    ///
    /// Never blocks. The work happens on [`Worker`]; this only decides whether
    /// to ask for it.
    pub fn update_preview(&mut self) {
        let Some(spec) = self.preview_spec() else {
            self.preview = Preview::Failed("this project declares no variants".to_owned());
            self.desired = None;
            return;
        };

        if self.desired.as_ref() != Some(&spec) {
            self.desired = Some(spec);
            self.changed_at = Instant::now();
        }

        // The debounce is the only thing separating "the selection moved" from
        // "the selection settled". Everything else is decided by `wanted`.
        if self.changed_at.elapsed() >= DEBOUNCE
            && let Some(spec) = self.wanted()
        {
            self.start_render(spec);
        }
    }

    /// The render that ought to be running, if it is not already.
    fn wanted(&self) -> Option<RenderSpec> {
        let desired = self.desired.as_ref()?;
        let on_screen = self.rendered_from.as_ref() == Some(desired);
        let under_way = self
            .in_flight
            .as_ref()
            .is_some_and(|(_, spec, _)| spec == desired);

        (!on_screen && !under_way).then(|| desired.clone())
    }

    /// Hand a specification to the worker.
    fn start_render(&mut self, spec: RenderSpec) {
        self.sequence = self.sequence.wrapping_add(1);
        if self.worker.request(self.sequence, spec.clone()) {
            self.in_flight = Some((self.sequence, spec, Instant::now()));
        } else {
            self.preview = Preview::Failed("the renderer stopped".to_owned());
        }
    }

    /// Take delivery of any finished render.
    ///
    /// Returns whether anything changed, so the caller can tell a frame worth
    /// drawing from one that is not.
    pub fn collect_preview(&mut self) -> bool {
        let mut changed = false;
        while let Some(rendered) = self.worker.collect() {
            changed |= self.apply(rendered);
        }
        changed
    }

    /// Show a finished render, if it is still the one being waited for.
    ///
    /// An answer to a superseded request is stale by definition, and showing
    /// it would flick the preview back to something the selection has already
    /// left.
    fn apply(&mut self, rendered: Rendered) -> bool {
        let Some((seq, spec, _)) = self.in_flight.as_ref() else {
            return false;
        };
        if *seq != rendered.seq {
            return false;
        }

        let spec = spec.clone();
        self.last_render = Some(rendered.elapsed);
        self.preview = match rendered.image {
            Ok(image) => Preview::Ready {
                image: Box::new(image),
                caption: format!(
                    "{} — {}x{} on {}",
                    spec.variant, spec.width, spec.height, spec.background
                ),
            },
            Err(reason) => Preview::Failed(reason),
        };
        self.generation = self.generation.wrapping_add(1);
        self.rendered_from = Some(spec);
        self.in_flight = None;
        true
    }

    /// Ask for a preview and wait for it.
    ///
    /// The blocking form, for the first frame and for tests. Bypasses the
    /// debounce, which exists to absorb a stream of keypresses and has nothing
    /// to absorb here.
    pub fn refresh_preview(&mut self) {
        self.update_preview();
        // Started regardless of the debounce, which exists to absorb a stream
        // of keypresses and has nothing to absorb here.
        if let Some(spec) = self.wanted() {
            self.start_render(spec);
        }

        while self.is_rendering() {
            // Applied through the same path a frame uses, so the blocking form
            // cannot diverge from the asynchronous one.
            match self.worker.wait(Duration::from_secs(30)) {
                Some(rendered) => {
                    self.apply(rendered);
                }
                None => {
                    self.preview = Preview::Failed("the renderer stopped answering".to_owned());
                    self.in_flight = None;
                    return;
                }
            }
        }
    }

    /// How many rows a pane would like, borders included.
    ///
    /// A list wants one row per entry; the prompt is prose and wants as much
    /// as it can get, so it reports a floor rather than a size.
    #[must_use]
    pub fn natural_height(&self, focus: Focus) -> u16 {
        match focus {
            // Prose has no natural height: it wants every row it can have, so
            // it reports the maximum and lets the caller cap it. Returning
            // "no rows, plus borders" instead made the collapsed prompt two
            // rows of border with the text entirely hidden.
            Focus::Prompt => u16::MAX,
            _ => u16::try_from(self.pane_len(focus))
                .unwrap_or(u16::MAX)
                .saturating_add(2),
        }
    }

    /// Record where the preview was drawn, and how big that is in pixels.
    ///
    /// The pixel size is what [`App::preview_spec`] fits the render to. It
    /// comes from the backend, because only the backend knows the terminal's
    /// cell size.
    pub(crate) fn set_preview_area(&mut self, area: Rect, cell: (u16, u16)) {
        self.preview_area = area;
        self.preview_pixels = (
            u32::from(area.width) * u32::from(cell.0),
            u32::from(area.height) * u32::from(cell.1),
        );
    }

    /// Where the preview was drawn last frame.
    #[must_use]
    pub const fn preview_area(&self) -> Rect {
        self.preview_area
    }

    /// Record where a pane was drawn.
    pub(crate) fn set_area(&mut self, focus: Focus, area: Rect) {
        self.areas[Self::index_of(focus)] = area;
    }

    /// Where a pane was drawn last frame.
    #[must_use]
    pub fn area(&self, focus: Focus) -> Rect {
        self.areas[Self::index_of(focus)]
    }

    /// The list scroll state for a pane, synchronised to its selection.
    pub(crate) fn list_state(&mut self, focus: Focus) -> &mut ListState {
        let index = Self::index_of(focus);
        let len = self.pane_len(focus);
        let selected = self.selected[index].min(len.saturating_sub(1));
        let state = &mut self.list_states[index];
        // Kept in step here rather than at every point that moves the
        // selection: ratatui scrolls to whatever the state says, so the state
        // is the thing that has to be right at draw time.
        state.select((len > 0).then_some(selected));
        state
    }

    /// The pane containing a point, if any.
    #[must_use]
    pub fn pane_at(&self, column: u16, row: u16) -> Option<Focus> {
        Focus::ALL.into_iter().find(|focus| {
            self.area(*focus)
                .contains(ratatui::layout::Position::new(column, row))
        })
    }

    /// Select the row a click landed on.
    ///
    /// Returns whether anything was selected. A click on a pane's border or
    /// past the end of its list focuses the pane without moving the cursor,
    /// which is what makes clicking a title bar harmless.
    pub fn select_at(&mut self, focus: Focus, row: u16) -> bool {
        let area = self.area(focus);
        // One row of border at the top, and the list's own scroll offset.
        let Some(offset) = row.checked_sub(area.y + 1) else {
            return false;
        };
        let index = usize::from(offset) + self.list_states[Self::index_of(focus)].offset();
        if index >= self.pane_len(focus) {
            return false;
        }
        self.selected[Self::index_of(focus)] = index;
        true
    }

    /// Move the description column's edge, keeping both columns usable.
    pub fn resize_column(&mut self, column: u16, total_width: u16) {
        let most = total_width.saturating_sub(MIN_PREVIEW).max(MIN_COLUMN);
        self.column_width = column.clamp(MIN_COLUMN, most);
    }

    /// Note a click, reporting whether it is a repeat of the last one.
    ///
    /// "Same place, soon after" rather than a count: a double-click that moved
    /// to a different row is two deliberate selections, not one gesture.
    pub fn register_click(&mut self, focus: Focus, row: u16) -> bool {
        let now = Instant::now();
        let repeat = self.last_click.is_some_and(|(previous, at, when)| {
            previous == focus && at == row && now.duration_since(when) < DOUBLE_CLICK
        });
        // Cleared on a repeat, so a third click starts a new gesture rather
        // than firing the action again.
        self.last_click = (!repeat).then_some((focus, row, now));
        repeat
    }

    /// Begin dragging the column divider.
    pub const fn begin_resize(&mut self) {
        self.resizing = true;
    }

    /// Stop dragging the column divider.
    pub const fn end_resize(&mut self) {
        self.resizing = false;
    }

    /// Whether the column divider is being dragged.
    #[must_use]
    pub const fn is_resizing(&self) -> bool {
        self.resizing
    }

    /// The full width of the last frame.
    #[must_use]
    pub const fn total_width(&self) -> u16 {
        self.column_width + self.preview_area.width
    }

    /// Move the selection down in a named pane.
    pub fn select_next_in(&mut self, focus: Focus) {
        self.step(focus, 1);
    }

    /// Move the selection up in a named pane.
    pub fn select_previous_in(&mut self, focus: Focus) {
        self.step(focus, -1);
    }

    /// Move a pane's selection by one, wrapping.
    fn step(&mut self, focus: Focus, delta: isize) {
        let len = self.pane_len(focus);
        if len == 0 {
            return;
        }
        let index = Self::index_of(focus);
        let current = self.selected[index].min(len - 1);
        let next = (current as isize + delta).rem_euclid(len as isize);
        self.selected[index] = next as usize;
    }

    /// Render the selected specification and write it to `dist/`.
    ///
    /// The result goes to the status line rather than being returned: an
    /// export that fails must not close the workspace, and a project mid-edit
    /// fails often.
    pub fn export_selected_render(&mut self) {
        let Some(spec) = self
            .project
            .metadata()
            .renders
            .get(self.selection(Focus::Renders))
            .cloned()
        else {
            return;
        };

        let directory = std::path::Path::new(EXPORT_DIRECTORY);
        self.notice = Some(
            match Renderer::new(&self.project, RenderOptions::default())
                .and_then(|renderer| renderer.render(&spec))
                .and_then(|asset| asset.write_to(directory))
            {
                Ok(path) => format!("wrote {}", path.display()),
                Err(error) => format!("export failed: {error}"),
            },
        );
    }

    /// Which image the preview currently holds.
    ///
    /// Changes whenever the pixels do, and never otherwise.
    #[must_use]
    pub const fn preview_generation(&self) -> u64 {
        self.generation
    }

    /// The current preview.
    #[must_use]
    pub fn preview(&self) -> &Preview {
        &self.preview
    }

    /// The current preview's pixels, if it has any.
    #[must_use]
    pub fn preview_image(&self) -> Option<&Image> {
        match &self.preview {
            Preview::Ready { image, .. } => Some(image),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;

    fn app() -> App {
        App::new(fixtures::project(), "blocks")
    }

    #[test]
    fn each_pane_remembers_its_own_selection() {
        // Otherwise moving through the variants would silently move the
        // palette's cursor too, and the preview would jump on the way back.
        let mut app = app();

        app.focus = Focus::Variants;
        app.select_next();
        app.focus = Focus::Palette;
        assert_eq!(app.selection(Focus::Palette), 0);
        assert_eq!(app.selection(Focus::Variants), 1);
    }

    #[test]
    fn the_selection_wraps_at_both_ends_of_a_pane() {
        let mut app = app();
        app.focus = Focus::Variants;

        app.select_previous();
        assert_eq!(
            app.selected_variant(),
            1,
            "moving up from the first wraps to the last"
        );
        app.select_next();
        assert_eq!(
            app.selected_variant(),
            0,
            "moving down from the last wraps to the first"
        );
    }

    #[test]
    fn moving_the_selection_in_a_pane_with_no_rows_does_nothing() {
        let mut app = app();
        app.focus = Focus::Prompt;
        app.select_next();
        assert_eq!(app.selection(Focus::Prompt), 0);
    }

    #[test]
    fn a_selection_left_beyond_the_end_of_a_shrunken_pane_is_clamped() {
        // Editing will make panes shrink under a cursor; an unclamped index
        // would panic during drawing, taking the terminal down with it.
        let mut app = app();
        app.focus = Focus::Variants;
        app.select_next();
        app.project.metadata_mut().variants.truncate(1);

        assert_eq!(app.selected_variant(), 0);
    }

    #[test]
    fn selecting_a_render_specification_previews_that_specification_exactly() {
        let mut app = app();
        app.focus = Focus::Renders;
        app.select_next();

        let spec = app.preview_spec().unwrap();
        assert_eq!(spec.name, "banner");
        assert_eq!((spec.width, spec.height), (128, 32));
    }

    #[test]
    fn selecting_a_variant_previews_it_at_a_default_size() {
        let mut app = app();
        app.focus = Focus::Variants;
        assert_eq!(app.preview_spec().unwrap().variant, "icon");
    }

    #[test]
    fn the_preview_is_rendered_once_and_then_cached() {
        let mut app = app();
        app.refresh_preview();
        assert!(matches!(app.preview(), Preview::Ready { .. }));

        // A second refresh with nothing moved must not re-rasterise.
        let before = app.preview().clone();
        app.refresh_preview();
        assert_eq!(app.preview(), &before);
    }

    #[test]
    fn moving_the_selection_makes_the_preview_follow() {
        let mut app = app();
        app.focus = Focus::Renders;
        app.refresh_preview();
        let Preview::Ready { image, .. } = app.preview().clone() else {
            panic!("expected a preview");
        };
        assert_eq!((image.width(), image.height()), (32, 32));

        app.select_next();
        app.refresh_preview();
        let Preview::Ready { image, .. } = app.preview().clone() else {
            panic!("expected a preview");
        };
        assert_eq!((image.width(), image.height()), (128, 32));
    }

    /// Pretend a pane of `cells` at a 10x20 cell size has been drawn.
    fn with_pane(app: &mut App, columns: u16, rows: u16) {
        app.set_preview_area(Rect::new(0, 0, columns, rows), (10, 20));
    }

    #[test]
    fn a_preview_is_halved_until_it_fits_the_pane() {
        // `social` is 1280x640. In a 58x27 pane at 10x20 px per cell — 580x540
        // — it must not be rasterised at its declared size, which is 819 000
        // pixels for a few thousand cells' worth of screen.
        let mut app = app();
        app.project.metadata_mut().renders.clear();
        app.project.metadata_mut().renders.push(RenderSpec {
            name: "social".to_owned(),
            variant: "icon".to_owned(),
            width: 1280,
            height: 640,
            format: Format::Png,
            background: crate::project::Background::Transparent,
        });
        app.focus = Focus::Renders;
        with_pane(&mut app, 58, 27);

        let spec = app.preview_spec().unwrap();
        assert_eq!((spec.width, spec.height), (320, 160));
    }

    #[test]
    fn fitting_a_preview_preserves_its_aspect_ratio_exactly() {
        // Halving is used precisely so the ratio survives. Scaling each side
        // to fit independently would letterbox an asset that does not.
        let mut app = app();
        app.project.metadata_mut().renders.clear();
        app.project.metadata_mut().renders.push(RenderSpec {
            name: "wide".to_owned(),
            variant: "icon".to_owned(),
            width: 1600,
            height: 400,
            format: Format::Png,
            background: crate::project::Background::Transparent,
        });
        app.focus = Focus::Renders;
        with_pane(&mut app, 30, 10);

        let spec = app.preview_spec().unwrap();
        assert_eq!(spec.width / spec.height, 4, "1600x400 is 4:1");
        assert!(spec.width <= 300, "should have shrunk: {}", spec.width);
    }

    #[test]
    fn a_preview_is_never_enlarged_to_fill_the_pane() {
        // A preview must not invent detail the asset does not have.
        let mut app = app();
        app.focus = Focus::Variants;
        with_pane(&mut app, 200, 60);

        let spec = app.preview_spec().unwrap();
        assert_eq!((spec.width, spec.height), (PREVIEW_SIZE, PREVIEW_SIZE));
    }

    #[test]
    fn a_preview_stops_shrinking_before_it_becomes_unrecognisable() {
        let mut app = app();
        app.focus = Focus::Variants;
        with_pane(&mut app, 1, 1);

        let spec = app.preview_spec().unwrap();
        assert!(spec.width >= MIN_PREVIEW_SIZE, "shrank to {}", spec.width);
    }

    #[test]
    fn before_the_first_frame_a_preview_uses_its_declared_size() {
        // `preview_pixels` is `(0, 0)` until something has been drawn, and a
        // zero-sized pane must not be read as "shrink to nothing".
        let mut app = app();
        app.focus = Focus::Variants;
        assert_eq!(app.preview_spec().unwrap().width, PREVIEW_SIZE);
    }

    #[test]
    fn resizing_the_pane_eventually_re_renders_the_preview() {
        // The fitted size is part of the cache key, so growing the pane has to
        // produce a better preview rather than a stretched old one.
        let mut app = app();
        app.focus = Focus::Variants;
        with_pane(&mut app, 10, 5);
        app.refresh_preview();
        let small = app.preview_image().unwrap().width();

        with_pane(&mut app, 80, 40);
        app.refresh_preview();
        let large = app.preview_image().unwrap().width();

        assert!(large > small, "{small} then {large}");
    }

    #[test]
    fn an_svg_render_specification_is_previewed_by_rasterising_it() {
        // Otherwise the pane reads "not a PNG", which is true and useless.
        let mut app = app();
        app.project.metadata_mut().renders.clear();
        let mut spec = RenderSpec::square("vector", "icon", 64);
        spec.format = Format::Svg;
        app.project.metadata_mut().renders.push(spec);

        app.focus = Focus::Renders;
        assert_eq!(app.preview_spec().unwrap().format, Format::Png);

        app.invalidate_preview();
        app.refresh_preview();
        assert!(matches!(app.preview(), Preview::Ready { .. }));
    }

    #[test]
    fn a_render_in_progress_is_visible_as_a_spinner() {
        // `Preview::Pending` and the spinner were unreachable while rendering
        // was synchronous: the frame could only be drawn once the render had
        // already finished, so there was never anything to report.
        let mut app = app();
        assert!(app.spinner().is_none(), "nothing is rendering yet");

        app.update_preview();
        if let Some(spec) = app.wanted() {
            app.start_render(spec);
        }

        assert!(app.is_rendering());
        assert!(app.spinner().is_some(), "a running render must be visible");

        app.refresh_preview();
        assert!(!app.is_rendering());
        assert!(app.spinner().is_none());
    }

    #[test]
    fn the_spinner_advances_over_time() {
        let mut app = app();
        app.update_preview();
        if let Some(spec) = app.wanted() {
            app.start_render(spec);
        }
        // Backdated rather than slept: a test that waits for an animation is a
        // test that is slow and flaky for no benefit.
        let first = app.spinner().unwrap();
        if let Some((_, _, started)) = app.in_flight.as_mut() {
            *started = Instant::now() - Duration::from_millis(200);
        }
        assert_ne!(first, app.spinner().unwrap());
    }

    #[test]
    fn a_selection_that_is_still_moving_does_not_start_a_render() {
        // Arrowing through a list must render the entry you stop on, not every
        // entry you pass over — each one costs a rasterise and, under Kitty, a
        // megabyte of terminal traffic.
        let mut app = app();
        app.refresh_preview();
        app.focus = Focus::Renders;

        app.select_next();
        app.update_preview();
        assert!(
            !app.is_rendering(),
            "a render started before the selection settled"
        );

        app.select_next();
        app.update_preview();
        assert!(!app.is_rendering());

        // Once it settles, it renders.
        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert!(app.is_rendering());
    }

    #[test]
    fn a_burst_of_keys_costs_one_render_not_one_per_key() {
        // The event loop drains everything that has arrived before drawing, so
        // three queued arrow keys reach this as three selection moves and one
        // decision. Under Kitty each render is over a megabyte of terminal
        // traffic, so the difference is three megabytes nobody would see.
        let mut app = app();
        app.refresh_preview();
        let after_initial = app.sequence;

        app.focus = Focus::Variants;
        for _ in 0..3 {
            app.select_next();
        }

        app.update_preview();
        assert_eq!(
            app.sequence, after_initial,
            "nothing should start while the selection is still moving"
        );

        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert_eq!(
            app.sequence,
            after_initial + 1,
            "the whole burst should cost exactly one render"
        );
    }

    #[test]
    fn returning_to_what_is_already_shown_costs_nothing() {
        // Arrowing down and back up again lands on the image already on
        // screen. Re-rendering it would be a megabyte for no change at all.
        let mut app = app();
        app.refresh_preview();
        let after_initial = app.sequence;

        app.focus = Focus::Variants;
        app.select_next();
        app.select_previous();

        app.changed_at = Instant::now() - DEBOUNCE;
        app.update_preview();
        assert_eq!(app.sequence, after_initial);
        assert!(!app.is_rendering());
    }

    #[test]
    fn a_stale_answer_is_discarded_rather_than_shown() {
        // The worker answers requests in order, but a request can be
        // superseded while it is running. Showing its answer would flick the
        // preview back to something the selection has already left.
        let mut app = app();
        app.refresh_preview();
        let before = app.preview().clone();

        let stale = Rendered {
            seq: app.sequence.wrapping_add(999),
            image: Ok(Image::from_rgba(1, 1, vec![1, 2, 3, 4])),
            elapsed: Duration::ZERO,
        };
        assert!(!app.apply(stale));
        assert_eq!(app.preview(), &before);
    }

    #[test]
    fn a_project_that_cannot_be_rendered_shows_the_reason_instead_of_quitting() {
        // A project mid-edit is often broken. Taking the workspace down on
        // every intermediate state would make it useless for editing.
        let mut app = app();
        app.project.metadata_mut().variants.clear();
        app.invalidate_preview();
        app.refresh_preview();

        assert!(matches!(app.preview(), Preview::Failed(_)));
    }
}
