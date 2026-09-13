//! Turning a project and a specification into an asset.
//!
//! ```text
//! project SVG -> parse -> variant isolation -> render spec -> PNG/SVG
//! ```
//!
//! Nothing in this module knows that an LLM exists, and nothing in it reads
//! the clock or the environment — and it never reaches the network itself:
//! a font declared by URL (ADR 015) is resolved, checksum-verified and
//! cached by [`crate::fonts`] before its bytes ever arrive here, the one
//! narrow, opt-in exception to "no network" that exists anywhere in Shaipe.
//! Given the same project bytes, the same specification and the same
//! already-resolved font bytes, this produces the same output bytes, on any
//! machine — which is the property that lets `shaipe render` be a CI check
//! rather than a convenience.
//!
//! It also knows nothing about the terminal: a render is bytes, and displaying
//! them is [`crate::preview`]'s problem.

pub mod asset;
pub mod fonts;
mod isolate;

pub use asset::RenderedAsset;
pub use fonts::FontPolicy;

use tiny_skia::{Pixmap, Transform};

use crate::error::{Error, Result};
use crate::project::Project;
use crate::project::document;
use crate::project::spec::{Format, RenderSpec};

/// How to render, as opposed to what to render.
///
/// Deliberately not part of a [`RenderSpec`]: a specification is a property of
/// the project and belongs in the file, whereas this is a property of the
/// invocation and belongs on the command line. Putting `--strict-fonts` in the
/// project would make it a thing a project could disable, which is exactly
/// backwards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderOptions {
    /// What to do about fonts the project does not supply.
    pub fonts: FontPolicy,
}

/// A project prepared for rendering.
///
/// Holds the assembled font database so that rendering a project's twelve
/// declared specifications builds it once rather than twelve times.
pub struct Renderer<'a> {
    project: &'a Project,
    options: usvg::Options<'static>,
}

impl<'a> Renderer<'a> {
    /// Prepare a project for rendering.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Font`] if a declared font file cannot be read, and
    /// [`Error::StrictFonts`] if a family is unresolved under
    /// [`FontPolicy::Strict`].
    pub fn new(project: &'a Project, options: RenderOptions) -> Result<Self> {
        let source = project.source().to_owned();
        let fontdb = {
            let parsed = document::parse(&source, project.path())?;
            fonts::database(project, &parsed, options.fonts)?
        };

        Ok(Self {
            project,
            options: usvg::Options {
                // So that `<image href="logo-mark.png">` resolves against the
                // project rather than against the working directory, which is
                // what makes `shaipe render` behave the same from anywhere.
                resources_dir: Some(project.base_directory()),
                fontdb,
                ..usvg::Options::default()
            },
        })
    }

    /// The project being rendered.
    #[must_use]
    pub fn project(&self) -> &Project {
        self.project
    }

    /// Render one specification.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSize`] for a degenerate canvas,
    /// [`Error::UnknownVariant`] or [`Error::UnknownElement`] if the
    /// specification points at something that is not there,
    /// [`Error::ParseSvg`] if the document does not survive isolation, and
    /// [`Error::Encode`] if the pixmap will not encode.
    pub fn render(&self, spec: &RenderSpec) -> Result<RenderedAsset> {
        if spec.width == 0 || spec.height == 0 {
            return Err(Error::InvalidSize {
                width: spec.width,
                height: spec.height,
            });
        }

        let source = self.project.source().to_owned();
        let isolated = {
            let parsed = document::parse(&source, self.project.path())?;
            isolate::isolate(self.project, &parsed, spec)?
        };

        let bytes = match spec.format {
            // The isolated document already carries the right root size and
            // viewBox, so it is the SVG output, unrasterised. A trailing
            // newline because it is a text file that gets committed: without
            // one, every `end-of-file-fixer` hook appends it and the next
            // render takes it away again, forever.
            Format::Svg => format!("{isolated}\n").into_bytes(),
            Format::Png => {
                let pixmap = self.rasterise(&isolated, spec)?;
                pixmap.encode_png().map_err(|source| Error::Encode {
                    spec: spec.name.clone(),
                    source,
                })?
            }
        };

        Ok(RenderedAsset {
            spec: spec.clone(),
            bytes,
        })
    }

    /// The isolated variant a specification names, as an SVG document.
    ///
    /// The step before either output: what [`Renderer::render`] returns
    /// verbatim for [`Format::Svg`], and what [`Renderer::pixels`] rasterises.
    ///
    /// # Errors
    ///
    /// As [`Renderer::render`], minus the encoding and the rasterising.
    pub fn isolated(&self, spec: &RenderSpec) -> Result<String> {
        let source = self.project.source().to_owned();
        let parsed = document::parse(&source, self.project.path())?;
        isolate::isolate(self.project, &parsed, spec)
    }

    /// What this variant is made of, as bytes to compare against later.
    ///
    /// The cheapest honest answer to "did *this* variant change?" — the
    /// workspace takes one either side of an edit and only re-renders when
    /// they differ, so an agent rewriting the wordmark does not cost a
    /// rasterise of the icon on screen. Not [`Renderer::isolated`], which
    /// carries every definition the document has and so changes whenever any
    /// variant does.
    ///
    /// Never rasterised, and never written: it is a comparison, not an asset.
    ///
    /// # Errors
    ///
    /// As [`Renderer::isolated`].
    pub fn fingerprint(&self, spec: &RenderSpec) -> Result<String> {
        let source = self.project.source().to_owned();
        let parsed = document::parse(&source, self.project.path())?;
        isolate::fingerprint(self.project, &parsed, spec)
    }

    /// Rasterise a specification, stopping at the pixels.
    ///
    /// The step before [`Renderer::render`] encodes anything. Callers that
    /// want to *show* a render rather than write one — the workspace — take
    /// this and skip a PNG encode and an immediate decode, which together cost
    /// more than the rasterising did.
    ///
    /// # Errors
    ///
    /// As [`Renderer::render`], minus the encoding.
    pub fn pixels(&self, spec: &RenderSpec) -> Result<Pixmap> {
        if spec.width == 0 || spec.height == 0 {
            return Err(Error::InvalidSize {
                width: spec.width,
                height: spec.height,
            });
        }

        let source = self.project.source().to_owned();
        let isolated = {
            let parsed = document::parse(&source, self.project.path())?;
            isolate::isolate(self.project, &parsed, spec)?
        };
        self.rasterise(&isolated, spec)
    }

    /// Rasterise an isolated variant onto its canvas.
    fn rasterise(&self, isolated: &str, spec: &RenderSpec) -> Result<Pixmap> {
        let tree =
            usvg::Tree::from_str(isolated, &self.options).map_err(|source| Error::ParseSvg {
                path: self.project.path().to_path_buf(),
                source,
            })?;

        let mut pixmap = Pixmap::new(spec.width, spec.height).ok_or(Error::InvalidSize {
            width: spec.width,
            height: spec.height,
        })?;

        // A transparent background is the absence of a fill, not a fill with
        // alpha zero: `Pixmap::new` already zeroes, and filling with a
        // transparent colour would be a no-op anyway.
        if let crate::project::spec::Background::Colour(colour) = spec.background {
            pixmap.fill(tiny_skia::Color::from_rgba8(
                colour.r, colour.g, colour.b, colour.a,
            ));
        }

        // Identity: the isolated document's root `viewBox` and `width`/`height`
        // already express the fit, so `usvg` has computed the scaling and
        // centring and there is nothing left to apply here.
        resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());

        Ok(pixmap)
    }

    /// Render every specification the project declares.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NothingToRender`] when there are none — a silent
    /// success that wrote no files is the least useful outcome of a CI step —
    /// and otherwise whatever [`Renderer::render`] returns, at the first
    /// specification that fails.
    pub fn render_declared(&self) -> Result<Vec<RenderedAsset>> {
        self.project
            .declared_renders()?
            .iter()
            .map(|spec| self.render(spec))
            .collect()
    }
}

/// Render one specification from a project.
///
/// The whole renderer in one call. Prefer [`Renderer`] when rendering more
/// than one specification from the same project, which reuses the font
/// database rather than rebuilding it each time.
///
/// # Errors
///
/// As [`Renderer::new`] and [`Renderer::render`].
pub fn render(project: &Project, spec: &RenderSpec) -> Result<RenderedAsset> {
    Renderer::new(project, RenderOptions::default())?.render(spec)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures::project as fixture;
    use crate::project::Rgba;
    use crate::project::spec::Background;

    /// The pixel at `(x, y)` of a rendered PNG, as straight RGBA.
    fn pixel(asset: &RenderedAsset, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let pixmap = Pixmap::decode_png(&asset.bytes).unwrap();
        let pixel = pixmap.pixel(x, y).unwrap();
        (pixel.red(), pixel.green(), pixel.blue(), pixel.alpha())
    }

    #[test]
    fn rendering_a_declared_specification_produces_a_png_of_the_requested_size() {
        let project = fixture();
        let spec = project.metadata().render_spec("favicon-32").unwrap();
        let asset = render(&project, spec).unwrap();

        assert_eq!(asset.file_name(), "favicon-32.png");
        let pixmap = Pixmap::decode_png(&asset.bytes).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (32, 32));
    }

    #[test]
    fn rendering_draws_the_variant_and_nothing_else() {
        // The icon symbol is a solid accent-coloured square filling its own
        // viewBox, so every pixel of a square canvas must be that colour.
        let project = fixture();
        let spec = project.metadata().render_spec("favicon-32").unwrap();
        let asset = render(&project, spec).unwrap();

        assert_eq!(pixel(&asset, 16, 16), (0xf0, 0x50, 0x32, 0xff));
        assert_eq!(pixel(&asset, 0, 0), (0xf0, 0x50, 0x32, 0xff));
    }

    #[test]
    fn a_transparent_background_leaves_uncovered_pixels_transparent() {
        // The wordmark is 4:1; on a square canvas it letterboxes, and the
        // bands above and below it must keep their alpha.
        let project = fixture();
        let spec = RenderSpec::square("probe", "wordmark", 64);
        let asset = render(&project, &spec).unwrap();

        assert_eq!(pixel(&asset, 32, 32), (0xf0, 0x50, 0x32, 0xff));
        assert_eq!(pixel(&asset, 32, 2), (0, 0, 0, 0));
    }

    #[test]
    fn an_opaque_background_fills_the_pixels_the_variant_does_not_cover() {
        let project = fixture();
        let mut spec = RenderSpec::square("probe", "wordmark", 64);
        spec.background = Background::Colour(Rgba::new(0x18, 0x18, 0x1b, 0xff));
        let asset = render(&project, &spec).unwrap();

        assert_eq!(pixel(&asset, 32, 32), (0xf0, 0x50, 0x32, 0xff));
        assert_eq!(pixel(&asset, 32, 2), (0x18, 0x18, 0x1b, 0xff));
    }

    #[test]
    fn rendering_the_same_project_twice_produces_identical_bytes() {
        // The property the whole CI story rests on. If this ever fails, some
        // part of the pipeline has started reading the clock, the environment
        // or a hash seed.
        let project = fixture();
        let spec = project.metadata().render_spec("banner").unwrap();
        assert_eq!(
            render(&project, spec).unwrap(),
            render(&project, spec).unwrap()
        );
    }

    #[test]
    fn rendering_as_svg_produces_a_standalone_document_of_that_variant() {
        let project = fixture();
        let mut spec = RenderSpec::square("mark", "icon", 512);
        spec.format = Format::Svg;
        let asset = render(&project, &spec).unwrap();

        let svg = String::from_utf8(asset.bytes).unwrap();
        assert!(
            svg.ends_with(">\n"),
            "an SVG asset is a text file and must end with a newline"
        );
        let svg = svg.trim_end().to_owned();
        assert_eq!(asset.spec.file_name(), "mark.svg");
        assert!(svg.starts_with("<svg"), "{svg}");
        assert!(svg.contains(r#"width="512" height="512""#), "{svg}");
        // And it must itself be a valid, renderable SVG.
        usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
    }

    #[test]
    fn a_zero_sized_canvas_is_refused_rather_than_producing_an_empty_file() {
        let project = fixture();
        let spec = RenderSpec::square("nothing", "icon", 0);
        assert!(matches!(
            render(&project, &spec).unwrap_err(),
            Error::InvalidSize { .. }
        ));
    }

    #[test]
    fn rendering_all_declared_specifications_produces_one_asset_each() {
        let project = fixture();
        let renderer = Renderer::new(&project, RenderOptions::default()).unwrap();
        let assets = renderer.render_declared().unwrap();

        assert_eq!(
            assets
                .iter()
                .map(RenderedAsset::file_name)
                .collect::<Vec<_>>(),
            ["favicon-32.png", "banner.png"]
        );
    }

    #[test]
    fn a_project_declaring_no_renders_says_so_instead_of_succeeding_silently() {
        let mut project = fixture();
        project.metadata_mut().renders.clear();
        let renderer = Renderer::new(&project, RenderOptions::default()).unwrap();

        assert!(matches!(
            renderer.render_declared().unwrap_err(),
            Error::NothingToRender { .. }
        ));
    }
}
