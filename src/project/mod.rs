//! A Shaipe project: an SVG file that is its own source of truth.
//!
//! The document holds the artwork; a `<shaipe:project>` element inside its
//! `<metadata>` holds everything else — the prompt, the palette, the fonts,
//! the variants, the references and the assets to produce. Nothing lives
//! beside the file. Copying `logo.svg` copies the project.
//!
//! The document stays a valid, ordinary SVG throughout. Its root draws the
//! primary variant, so it renders in a browser and on GitHub like any other
//! logo, and every tool that does not know about Shaipe simply ignores a
//! namespace it has never heard of.

pub mod document;
pub mod font;
pub mod metadata;
pub mod palette;
pub mod reference;
pub mod spec;
pub mod variant;

use std::path::{Path, PathBuf};

pub use document::SVG_NAMESPACE;
pub use font::Font;
pub use metadata::{Generation, Metadata};
pub use palette::{Colour, Hsl, Palette, Rgba, Role};
pub use reference::{Reference, ReferenceKind};
pub use spec::{Background, Format, RenderSpec};
pub use variant::Variant;

use crate::error::{Error, Result};

/// The bytes of a freshly created project, before its variant is declared.
///
/// A self-closing placeholder `<shaipe:project>` gives
/// [`document::replace_metadata`] a byte range to splice into — the same path
/// every other edit takes, which is what makes a freshly initialised project
/// round-trip identically once it is saved.
///
/// The root's `width`/`height` (512) are deliberately larger than its
/// `viewBox` (64) — they only set the intrinsic display size a browser or
/// image viewer uses when there is nothing else to size the file by
/// (`preserveAspectRatio` scales the `viewBox` content to fit uniformly, with
/// no effect on any coordinate a symbol is drawn in, and no effect on an
/// export — see `ADR-003`, which reads a `RenderSpec`'s own dimensions
/// instead). Left equal to `viewBox` before, every fresh project opened
/// exactly as large as its own 64-unit coordinate system, i.e. tiny.
const TEMPLATE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 64 64" width="512" height="512">
  <metadata>
    <shaipe:project version="1"/>
  </metadata>
  <symbol id="icon" viewBox="0 0 64 64">
    <rect width="64" height="64" fill="currentColor"/>
  </symbol>
  <use href="#icon" width="64" height="64"/>
</svg>
"##;

/// An opened Shaipe project.
///
/// Holds the original bytes as well as the parsed metadata. The bytes are what
/// the renderer parses and what a metadata edit splices into; keeping them
/// means Shaipe never has to reconstruct a document it did not write.
#[derive(Debug, Clone)]
pub struct Project {
    path: PathBuf,
    source: String,
    metadata: Metadata,
}

impl Project {
    /// Open a project file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be read, [`Error::NotSvg`] or
    /// [`Error::MalformedXml`] if it is not an SVG, and [`Error::NoMetadata`]
    /// or [`Error::InvalidMetadata`] if its Shaipe metadata is missing or
    /// unreadable.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let source = document::read(&path)?;
        Self::from_source(path, source)
    }

    /// Parse a project from bytes already in hand.
    ///
    /// `path` is used to resolve the project's relative references and to name
    /// the file in diagnostics; it is not read.
    ///
    /// # Errors
    ///
    /// As [`Project::open`], minus the I/O.
    pub fn from_source(path: impl Into<PathBuf>, source: String) -> Result<Self> {
        let path = path.into();
        // Scoped so the borrow of `source` ends before `source` is moved: the
        // parsed document borrows the bytes, the project owns them.
        let metadata = {
            let parsed = document::parse(&source, &path)?;
            Metadata::from_document(&parsed, &path)?
        };

        Ok(Self {
            path,
            source,
            metadata,
        })
    }

    /// Create a fresh project in memory: a placeholder icon, declared as the
    /// primary variant.
    ///
    /// Nothing is written to disk — this only produces the in-memory value
    /// that `shaipe init`, or a workspace opened on a path that does not yet
    /// exist, would go on to [`Project::save`].
    ///
    /// # Errors
    ///
    /// Only if the fixed template ever stopped parsing, which a unit test
    /// guards against; it should never happen in practice.
    pub fn init(path: impl Into<PathBuf>) -> Result<Self> {
        let mut project = Self::from_source(path, TEMPLATE.to_owned())?;
        project.metadata.primary = Some("icon".to_owned());
        project.metadata.variants.push(Variant::new("icon"));
        Ok(project)
    }

    /// Where the project file lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The directory relative paths in the metadata resolve against.
    #[must_use]
    pub fn base_directory(&self) -> PathBuf {
        document::base_directory(&self.path)
    }

    /// The document's bytes, exactly as they were read.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Everything the document says about itself.
    #[must_use]
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Everything the document says about itself, mutably.
    ///
    /// Changes are in memory until [`Project::save`] is called. That is the
    /// seam palette editing and agent-driven edits are built on.
    pub fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }

    /// The specifications a bare `shaipe render` would produce.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NothingToRender`] when the project declares none,
    /// rather than succeeding silently having written no files.
    pub fn declared_renders(&self) -> Result<&[RenderSpec]> {
        if self.metadata.renders.is_empty() {
            return Err(Error::NothingToRender {
                path: self.path.clone(),
            });
        }
        Ok(&self.metadata.renders)
    }

    /// Resolve a path from the metadata against the project's directory.
    ///
    /// An absolute path is returned as it stands, so a project can point at
    /// something outside its own tree when it must.
    #[must_use]
    pub fn resolve(&self, relative: &Path) -> PathBuf {
        if relative.is_absolute() {
            relative.to_path_buf()
        } else {
            self.base_directory().join(relative)
        }
    }

    /// Restyle every element bound to a palette colour, to a given value.
    ///
    /// Binding is `shaipe:fill`/`shaipe:stroke` naming a colour by its
    /// palette `name`; see [`document::apply_binding`]. This only touches the
    /// artwork — the palette's own record of the colour is a separate field
    /// on [`Metadata`], and updating it is the caller's job, the same way
    /// `set_palette_colour` and the workspace's palette pane both do.
    ///
    /// # Errors
    ///
    /// As [`document::apply_binding`] — unreachable in practice, since this
    /// project's own source already parsed once, when it was opened.
    pub fn restyle(&mut self, name: &str, value: Rgba) -> Result<()> {
        self.source = document::apply_binding(&self.source, name, value, &self.path)?;
        Ok(())
    }

    /// The bytes this project would be written as.
    ///
    /// Identical to [`Project::source`] apart from the metadata element, and
    /// identical to it entirely when the metadata has not been touched.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoMetadata`] if the source no longer parses, which
    /// should not be reachable for a project that opened successfully.
    pub fn to_svg(&self) -> Result<String> {
        document::replace_metadata(&self.source, &self.metadata, &self.path)
    }

    /// Write the project back to its file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be written.
    pub fn save(&mut self) -> Result<()> {
        let updated = self.to_svg()?;
        document::write(&self.path, &updated)?;
        self.source = updated;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    use crate::fixtures::{PROJECT as FIXTURE, project as fixture};

    #[test]
    fn opening_a_project_reads_every_part_of_its_metadata() {
        let project = fixture();
        let metadata = project.metadata();

        assert_eq!(metadata.primary.as_deref(), Some("icon"));
        assert_eq!(metadata.prompt.as_deref(), Some("A square and a bar."));
        assert_eq!(metadata.palette.len(), 2);
        assert_eq!(metadata.variant_names(), ["icon", "wordmark"]);
        assert_eq!(metadata.spec_names(), ["favicon-32", "banner"]);
    }

    #[test]
    fn a_variant_may_reference_an_element_of_a_different_name() {
        assert_eq!(
            fixture().metadata().variant("wordmark").unwrap().element,
            "mark-wide"
        );
    }

    #[test]
    fn a_render_declaring_only_a_width_is_square() {
        let project = fixture();
        let spec = project.metadata().render_spec("favicon-32").unwrap();
        assert_eq!((spec.width, spec.height), (32, 32));
    }

    #[test]
    fn asking_for_a_variant_that_does_not_exist_says_which_ones_do() {
        let project = fixture();
        let error = project.metadata().resolve_variant("watermark").unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("watermark"), "{rendered}");
        assert!(matches!(error, Error::UnknownVariant { .. }));
    }

    #[test]
    fn saving_an_untouched_project_would_not_change_a_byte() {
        // `shaipe` must be able to open a project without dirtying the working
        // tree, or the assets CI check is unusable.
        let project = fixture();
        assert_eq!(project.to_svg().unwrap(), FIXTURE);
    }

    #[test]
    fn a_document_without_shaipe_metadata_is_refused_with_an_explanation() {
        let plain = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"#;
        let error = Project::from_source("plain.svg", plain.to_owned()).unwrap_err();
        assert!(matches!(error, Error::NoMetadata { .. }));
    }

    #[test]
    fn a_project_from_a_future_schema_is_refused_rather_than_misread() {
        let future = FIXTURE.replace(r#"version="1""#, r#"version="2""#);
        let error = Project::from_source("logo.svg", future).unwrap_err();
        assert!(matches!(error, Error::UnsupportedSchema { .. }));
    }

    #[test]
    fn relative_metadata_paths_resolve_against_the_project_directory() {
        let project = Project::from_source("assets/brand/logo.svg", FIXTURE.to_owned()).unwrap();
        assert_eq!(
            project.resolve(Path::new("fonts/Inter.ttf")),
            Path::new("assets/brand/fonts/Inter.ttf")
        );
    }

    #[test]
    fn initialising_a_project_declares_one_primary_variant() {
        let project = Project::init("logo.svg").unwrap();
        assert_eq!(project.metadata().primary.as_deref(), Some("icon"));
        assert_eq!(project.metadata().variant_names(), ["icon"]);
    }

    #[test]
    fn restyling_a_bound_colour_changes_the_source_but_not_the_palettes_own_record() {
        // The two are kept independent on purpose: a tool or the workspace
        // decides when to update each, and this method only ever does the
        // first.
        const BOUND: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 16 16">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:palette>
        <shaipe:color name="accent" value="#f05032"/>
      </shaipe:palette>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
      </shaipe:variants>
    </shaipe:project>
  </metadata>
  <symbol id="icon" viewBox="0 0 16 16"><rect width="16" height="16" fill="#f05032" shaipe:fill="accent"/></symbol>
  <use href="#icon"/>
</svg>
"##;

        let mut project = Project::from_source("logo.svg", BOUND.to_owned()).unwrap();
        project.restyle("accent", Rgba::new(0, 0, 0, 0xff)).unwrap();

        assert!(project.source().contains(r##"fill="#000000""##));
        assert_eq!(
            project.metadata().palette.get("accent").unwrap().value,
            Rgba::new(0xf0, 0x50, 0x32, 0xff)
        );
    }

    #[test]
    fn saving_a_freshly_initialised_project_then_saving_it_again_changes_nothing() {
        // The same guarantee `saving_an_untouched_project_would_not_change_a_byte`
        // makes for an existing project, extended to one that never existed
        // before: once it has been saved once, saving it again must be a no-op.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");

        let mut project = Project::init(&path).unwrap();
        project.save().unwrap();
        let once = std::fs::read_to_string(&path).unwrap();

        project.save().unwrap();
        let twice = std::fs::read_to_string(&path).unwrap();

        assert_eq!(once, twice);
    }
}
