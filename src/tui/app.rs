//! The workspace's state.
//!
//! Deliberately plain: fields and small methods, no framework. The preview is
//! the only derived state, and it is cached because rendering on every frame
//! would rasterise a 512-pixel SVG four times a second for no reason.
//!
//! This is where editing will land. A prompt editor, a colour picker and an
//! agent conversation all need somewhere to hold a draft and a dirty flag, and
//! that somewhere is here rather than spread through the drawing code.

use crate::preview::Image;
use crate::project::{Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};

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

    selected: [usize; Focus::COUNT],
    preview: Preview,
    /// What the cached preview was rendered from. `None` forces a re-render.
    rendered_from: Option<RenderSpec>,
}

impl App {
    /// Open a project in the workspace.
    #[must_use]
    pub fn new(project: Project, backend: &'static str) -> Self {
        Self {
            project,
            focus: Focus::Variants,
            should_quit: false,
            backend,
            selected: [0; Focus::COUNT],
            preview: Preview::Pending,
            rendered_from: None,
        }
    }

    /// The index of the focused pane, for indexing `selected`.
    fn focus_index(&self) -> usize {
        Focus::ALL
            .iter()
            .position(|focus| *focus == self.focus)
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

        if self.focus == Focus::Renders
            && let Some(spec) = metadata.renders.get(self.selection(Focus::Renders))
        {
            return Some(spec.clone());
        }

        let variant = metadata.variants.get(self.selected_variant())?;
        Some(RenderSpec::square(&variant.name, &variant.name, 512))
    }

    /// Throw the cached preview away, so the next frame re-renders it.
    pub fn invalidate_preview(&mut self) {
        self.rendered_from = None;
    }

    /// Re-render the preview if the selection has moved since it was made.
    pub fn refresh_preview(&mut self) {
        let Some(spec) = self.preview_spec() else {
            self.preview = Preview::Failed("this project declares no variants".to_owned());
            return;
        };

        if self.rendered_from.as_ref() == Some(&spec) {
            return;
        }

        // Through the same renderer `shaipe render` uses, so the preview is
        // the asset rather than an impression of it.
        self.preview = match Renderer::new(&self.project, RenderOptions::default())
            .and_then(|renderer| renderer.render(&spec))
            .and_then(|asset| Image::from_asset(&asset).map(|image| (image, asset.spec.clone())))
        {
            Ok((image, spec)) => Preview::Ready {
                image: Box::new(image),
                caption: format!(
                    "{} — {}x{} on {}",
                    spec.variant, spec.width, spec.height, spec.background
                ),
            },
            Err(error) => Preview::Failed(error.to_string()),
        };

        self.rendered_from = Some(spec);
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
