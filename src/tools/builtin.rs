//! The tools Shaipe implements.
//!
//! All thin: each one is a name and a shape over an operation the library
//! already performs. That is the intended proportion — a tool that contained
//! logic of its own would be logic the CLI and the TUI could not reach.
//!
//! Most are read-only. The mutating ones — `write_svg`, `write_variant`,
//! `set_palette_colour`, `set_generation` — validate before anything is
//! replaced and change the project only in memory; see [`Tool::mutates`] for
//! what that means to a caller.

use serde_json::{Value, json};

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::inspect::Report;
use crate::project::document;
use crate::project::palette::{Colour, Role};
use crate::project::{Background, Format, Project, Reference, ReferenceKind, RenderSpec, Rgba};
use crate::render::{RenderOptions, Renderer};
use crate::tools::{
    Tool, ToolImage, ToolOutput, boolean, integer, integers, object, optional_u32, required_str,
    string,
};

/// Every tool Shaipe implements.
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(GetProject),
        Box::new(GetVariants),
        Box::new(GetPalette),
        Box::new(GetReferences),
        Box::new(GetReferenceImage),
        Box::new(GetReferenceTrace),
        Box::new(GetSvg),
        Box::new(RenderSvg),
        Box::new(RenderGrid),
        Box::new(WriteSvg),
        Box::new(WriteVariant),
        Box::new(SetPaletteColour),
        Box::new(SetReference),
        Box::new(SetGeneration),
    ]
}

/// The sizes `render_grid` draws when it is not told which.
///
/// Largest first, so a model reads the mark before it reads the smudge, and
/// down to 16 because that is a favicon and where legibility actually fails.
const DEFAULT_GRID: [u32; 5] = [512, 128, 64, 32, 16];

/// The most sizes `render_grid` will draw in one call.
///
/// An unbounded list is an unbounded quantity of image in one response, which
/// is a context window spent on the model's own typo.
const MAX_GRID: usize = 8;

/// Describe the whole project.
struct GetProject;

impl Tool for GetProject {
    fn name(&self) -> &'static str {
        "get_project"
    }

    fn description(&self) -> &'static str {
        "Describe a Shaipe project: its prompt, palette, fonts, variants, \
         references and the assets it declares. Call this first — it is the \
         only way to learn what the project's variants and colours are named."
    }

    fn input_schema(&self) -> Value {
        object(&[], &[])
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<ToolOutput> {
        Ok(ToolOutput::json(
            serde_json::to_value(Report::of(project)).expect("a report is always serialisable"),
        ))
    }
}

/// List the renderable parts of the document.
struct GetVariants;

impl Tool for GetVariants {
    fn name(&self) -> &'static str {
        "get_variants"
    }

    fn description(&self) -> &'static str {
        "List the project's variants: the named parts of the SVG that can be \
         rendered on their own, such as an icon or a wordmark, and the element \
         id in the document that draws each one."
    }

    fn input_schema(&self) -> Value {
        object(&[], &[])
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<ToolOutput> {
        let metadata = project.metadata();
        Ok(ToolOutput::json(Value::Array(
            metadata
                .variants
                .iter()
                .map(|variant| {
                    json!({
                        "name": variant.name,
                        "element": variant.element,
                        "primary": metadata.primary.as_deref() == Some(variant.name.as_str()),
                    })
                })
                .collect(),
        )))
    }
}

/// Read the palette.
struct GetPalette;

impl Tool for GetPalette {
    fn name(&self) -> &'static str {
        "get_palette"
    }

    fn description(&self) -> &'static str {
        "List the project's colours: each one's name, CSS hex value and \
         semantic role. Use these values rather than inventing colours when \
         editing the artwork."
    }

    fn input_schema(&self) -> Value {
        object(&[], &[])
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<ToolOutput> {
        Ok(ToolOutput::json(Value::Array(
            project
                .metadata()
                .palette
                .colours()
                .iter()
                .map(|colour| {
                    json!({
                        "name": colour.name,
                        "value": colour.value.to_string(),
                        "role": colour.role.as_ref().map(ToString::to_string),
                    })
                })
                .collect(),
        )))
    }
}

/// Read the attached references.
struct GetReferences;

impl Tool for GetReferences {
    fn name(&self) -> &'static str {
        "get_references"
    }

    fn description(&self) -> &'static str {
        "List the files attached to the project for context: images or \
         documents recorded as a source to reproduce, inspiration to take \
         cues from, or a baseline rendering to compare against. Each entry \
         reports whether the file actually exists."
    }

    fn input_schema(&self) -> Value {
        object(&[], &[])
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<ToolOutput> {
        Ok(ToolOutput::json(Value::Array(
            project
                .metadata()
                .references
                .iter()
                .map(|reference| {
                    let resolved = project.resolve(&reference.src);
                    json!({
                        "src": reference.src.display().to_string(),
                        "resolved": resolved.display().to_string(),
                        "kind": reference.kind.to_string(),
                        "note": reference.note,
                        "present": resolved.exists(),
                    })
                })
                .collect(),
        )))
    }
}

/// Read one attached reference's bytes and hand them to whoever is looking.
struct GetReferenceImage;

impl Tool for GetReferenceImage {
    fn name(&self) -> &'static str {
        "get_reference_image"
    }

    fn description(&self) -> &'static str {
        "Read an attached reference file's bytes and look at it. Use this \
         before tracing or matching a `source` reference — `get_references` \
         only reports whether the file exists, not what it shows. Supports \
         PNG, JPEG, GIF, WEBP and BMP."
    }

    fn input_schema(&self) -> Value {
        object(
            &[(
                "src",
                string(
                    "Which reference to look at. One of the paths from \
                     `get_references`, matched exactly.",
                ),
            )],
            &["src"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let src = PathBuf::from(required_str(self.name(), input, "src")?);

        let refuse = |reason: String| crate::Error::InvalidToolInput {
            tool: self.name().to_owned(),
            reason,
        };

        // Resolved before anything is read, so a name that was never attached
        // is reported without ever touching the filesystem.
        let reference = project
            .metadata()
            .references
            .iter()
            .find(|reference| reference.src == src)
            .cloned()
            .ok_or_else(|| {
                let known: Vec<String> = project
                    .metadata()
                    .references
                    .iter()
                    .map(|reference| reference.src.display().to_string())
                    .collect();
                refuse(format!(
                    "`{}` is not an attached reference. {}",
                    src.display(),
                    if known.is_empty() {
                        "Nothing is attached yet; call `set_reference` to attach one.".to_owned()
                    } else {
                        format!("Attached: {}", known.join(", "))
                    }
                ))
            })?;

        let resolved = project.resolve(&reference.src);

        // Missing or unreadable reads exactly like any other file-access
        // failure in the crate — `get_references`' `present` already told the
        // model whether this was likely, so a surprise here is a race, not a
        // guess.
        let bytes =
            std::fs::read(&resolved).map_err(|error| crate::Error::io(resolved.clone(), error))?;

        // Guessed from the extension, never sniffed from the bytes: Shaipe
        // reads nothing about the file's content here, the same way
        // `get_references`' `present` only checks existence. An agent that
        // attaches a mislabelled file gets a mislabelled image, which is its
        // mistake to notice, not Shaipe's to fix.
        let mime_type = reference_mime_type(&resolved).ok_or_else(|| {
            refuse(format!(
                "`{}` is not an image format this tool recognises; expected \
                 .png, .jpg, .jpeg, .gif, .webp or .bmp",
                resolved.display()
            ))
        })?;

        Ok(ToolOutput::json(json!({
            "src": reference.src.display().to_string(),
            "resolved": resolved.display().to_string(),
            "kind": reference.kind.to_string(),
            "note": reference.note,
            "mime_type": mime_type,
        }))
        .with_image(ToolImage::new(
            format!("{} ({})", reference.src.display(), reference.kind),
            mime_type,
            bytes,
        )))
    }
}

/// The raster formats `get_reference_image` will hand to an agent.
fn reference_mime_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

/// Trace an attached reference's pixels into vector paths, algorithmically.
///
/// See [`crate::vectorize`] and ADR 014 for why this exists at all: an LLM
/// writing `<path d="...">` from a prompt and an image can name the shapes
/// present but cannot reproduce an exact curve or corner radius from it —
/// that is a measurement, not a description, and this tool is what actually
/// measures.
struct GetReferenceTrace;

impl Tool for GetReferenceTrace {
    fn name(&self) -> &'static str {
        "get_reference_trace"
    }

    fn description(&self) -> &'static str {
        "Trace an attached reference image into vector paths, algorithmically \
         — not by describing the shape, by measuring its pixels. Use this \
         before hand-writing a mark from a `source` reference: an LLM can name \
         the shapes in an image but cannot reproduce exact curves, corner \
         radii or organic tapering from it; this can. Returns a small \
         standalone SVG — one silhouette, holes cut by winding, no colour \
         baked in — to inspect and then hand-graft into a variant with \
         `write_variant` or `write_svg`, the same way any other markup is \
         merged. Only separates one foreground colour from one background; \
         it is not for tracing photographs or multi-colour artwork. If the \
         result is many small fragments instead of one coherent shape, the \
         mark is probably a medium-brightness colour (a saturated green or \
         blue reads as roughly as bright as its background to the default \
         cutoff) rather than genuinely dark — raise `threshold` toward 200+ \
         and try again."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "src",
                    string(
                        "Which reference to trace. One of the paths from \
                         `get_references`, matched exactly.",
                    ),
                ),
                (
                    "threshold",
                    json!({
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 255,
                        "description": "Binary cutoff separating foreground from \
                         background: pixels darker than this are traced. Lower \
                         it if background is being traced instead of the mark, \
                         raise it if part of the mark is being missed. Omit for \
                         a sensible default (128).",
                    }),
                ),
                (
                    "invert",
                    boolean(
                        "Set when the reference is light artwork on a dark \
                         background, so light pixels are traced as the \
                         foreground instead of dark ones.",
                    ),
                ),
            ],
            &["src"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let src = PathBuf::from(required_str(self.name(), input, "src")?);
        let threshold = input
            .get("threshold")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok());
        let invert = input
            .get("invert")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let refuse = |reason: String| crate::Error::InvalidToolInput {
            tool: self.name().to_owned(),
            reason,
        };

        // Same resolution as `get_reference_image`: reported before anything
        // is read, so an unattached name is a clear refusal, not a race.
        let reference = project
            .metadata()
            .references
            .iter()
            .find(|reference| reference.src == src)
            .cloned()
            .ok_or_else(|| {
                let known: Vec<String> = project
                    .metadata()
                    .references
                    .iter()
                    .map(|reference| reference.src.display().to_string())
                    .collect();
                refuse(format!(
                    "`{}` is not an attached reference. {}",
                    src.display(),
                    if known.is_empty() {
                        "Nothing is attached yet; call `set_reference` to attach one.".to_owned()
                    } else {
                        format!("Attached: {}", known.join(", "))
                    }
                ))
            })?;

        let resolved = project.resolve(&reference.src);

        let bytes =
            std::fs::read(&resolved).map_err(|error| crate::Error::io(resolved.clone(), error))?;

        let svg = crate::vectorize::trace(
            &resolved,
            &bytes,
            crate::vectorize::TraceOptions { threshold, invert },
        )?;
        let path_count = svg.matches("<path").count();

        Ok(ToolOutput::json(json!({
            "src": reference.src.display().to_string(),
            "resolved": resolved.display().to_string(),
            "svg": svg,
            "path_count": path_count,
        })))
    }
}

/// Rasterise a variant.
struct RenderSvg;

impl Tool for RenderSvg {
    fn name(&self) -> &'static str {
        "render_svg"
    }

    fn description(&self) -> &'static str {
        "Render a variant of the project to a PNG and look at it. Rendering is \
         local and exact, so the image returned is the asset a build would \
         produce, not an approximation of it. This is the only way to find out \
         what the artwork actually looks like."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "variant",
                    string("Which variant to draw. One of the names from `get_variants`."),
                ),
                ("width", integer("Canvas width in pixels. Defaults to 512.")),
                (
                    "height",
                    integer("Canvas height in pixels. Defaults to the width, giving a square."),
                ),
                (
                    "background",
                    string(
                        "`transparent`, or a CSS hex colour such as `#ffffff`. \
                         Defaults to transparent — set a colour to check the \
                         mark holds up against it.",
                    ),
                ),
            ],
            &["variant"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let variant = required_str(self.name(), input, "variant")?;

        // A default rather than a required argument: a model asking to see a
        // logo has no basis for choosing a size, and being forced to invent
        // one adds a decision that can be wrong.
        let width = optional_u32(input, "width", 512);

        let spec = RenderSpec {
            name: variant.to_owned(),
            variant: variant.to_owned(),
            width,
            height: optional_u32(input, "height", width),
            format: Format::Png,
            background: background(self.name(), input)?,
        };

        render_one(project, &spec)
    }
}

/// Rasterise one spec into an answer and an image to look at.
///
/// Shared rather than inlined because two tools rasterise, and a second copy
/// is a second place for the reported size to drift from the encoded one.
fn render_one(project: &Project, spec: &RenderSpec) -> Result<ToolOutput> {
    let asset = Renderer::new(project, RenderOptions::default())?.render(spec)?;

    Ok(ToolOutput::json(json!({
        "variant": spec.variant,
        "width": spec.width,
        "height": spec.height,
    }))
    .with_image(ToolImage::png(
        format!("{} at {}x{}", spec.variant, spec.width, spec.height),
        asset.bytes,
    )))
}

/// Read the `background` argument, which both rendering tools accept.
fn background(tool: &str, input: &Value) -> Result<Background> {
    match input.get("background").and_then(Value::as_str) {
        None => Ok(Background::Transparent),
        // The parse error names the bad value but not the argument it came
        // from, and a model correcting itself needs both.
        Some(value) => {
            value
                .parse()
                .map_err(|error: crate::Error| crate::Error::InvalidToolInput {
                    tool: tool.to_owned(),
                    reason: format!("`background`: {error}"),
                })
        }
    }
}

/// Return the document itself.
struct GetSvg;

impl Tool for GetSvg {
    fn name(&self) -> &'static str {
        "get_svg"
    }

    fn description(&self) -> &'static str {
        "Return the project's SVG source exactly as it is, including the \
         `<metadata>` block that makes it a Shaipe project. Read this before \
         calling `write_svg`: an edit that was not based on the current source \
         will silently discard whatever else the file contains."
    }

    fn input_schema(&self) -> Value {
        object(&[], &[])
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<ToolOutput> {
        // The document as it now stands, not as it was read. `Project` keeps
        // the bytes and the parsed metadata side by side, and only saving
        // reconciles them — so `source()` still carries the old
        // `<shaipe:prompt>` while someone is editing it in the workspace.
        //
        // Serving that was a data-loss bug, not merely a stale read: the model
        // edits the document it was given and sends it back, `write_svg`
        // replaces the whole project from those bytes, and the prompt the user
        // had just written is silently reverted. It also disagreed with
        // `get_project`, which reports from metadata, in the same turn.
        //
        // Identical to `source()` for a project nobody has touched, because
        // writing omits defaults — the same rule that lets an untouched
        // project be saved without changing a byte.
        let source = project.to_svg()?;

        // Whole, never truncated. A model handed half a document writes back
        // half a document, and `write_svg` would then be right to reject it —
        // having spent a turn on a failure this tool caused.
        Ok(ToolOutput::json(json!({
            "path": project.path().display().to_string(),
            "bytes": source.len(),
            "source": source,
        })))
    }
}

/// Rasterise a variant at several sizes at once.
struct RenderGrid;

impl Tool for RenderGrid {
    fn name(&self) -> &'static str {
        "render_grid"
    }

    fn description(&self) -> &'static str {
        "Render one variant at several sizes at once and look at all of them. \
         Use this to check that a mark still reads when it is small: a logo \
         that works at 512 pixels often becomes an unreadable smudge at 16, \
         and the only way to know is to look at it at 16."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "variant",
                    string("Which variant to draw. One of the names from `get_variants`."),
                ),
                (
                    "sizes",
                    integers(
                        "The square sizes to render, in pixels. Defaults to \
                         512, 128, 64, 32 and 16.",
                        MAX_GRID,
                    ),
                ),
                (
                    "background",
                    string(
                        "`transparent`, or a CSS hex colour such as `#ffffff`. \
                         Defaults to transparent — set a colour to check the \
                         mark holds up against it.",
                    ),
                ),
            ],
            &["variant"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let variant = required_str(self.name(), input, "variant")?;
        let background = background(self.name(), input)?;
        let sizes = self.sizes(input)?;

        let renderer = Renderer::new(project, RenderOptions::default())?;

        // One image per size, at its true resolution — never composited into a
        // contact sheet. A 16-pixel mark pasted into a 512-wide sheet is
        // either upscaled, which is a lie about the one thing being checked,
        // or occupies a thousandth of the pixels the model is looking at.
        let mut images = Vec::with_capacity(sizes.len());
        for size in &sizes {
            let spec = RenderSpec {
                name: variant.to_owned(),
                variant: variant.to_owned(),
                width: *size,
                height: *size,
                format: Format::Png,
                background,
            };
            images.push(ToolImage::png(
                format!("{variant} at {size}x{size}"),
                renderer.render(&spec)?.bytes,
            ));
        }

        Ok(ToolOutput::json(json!({
            "variant": variant,
            "sizes": sizes,
            "background": background.to_string(),
        }))
        .with_images(images))
    }
}

impl RenderGrid {
    /// The sizes to draw, defaulted and bounded.
    fn sizes(&self, input: &Value) -> Result<Vec<u32>> {
        let Some(requested) = input.get("sizes") else {
            return Ok(DEFAULT_GRID.to_vec());
        };

        let refuse = |reason: String| crate::Error::InvalidToolInput {
            tool: self.name().to_owned(),
            reason,
        };

        let requested = requested
            .as_array()
            .ok_or_else(|| refuse("`sizes` must be an array of pixel sizes".to_owned()))?;

        if requested.is_empty() {
            return Ok(DEFAULT_GRID.to_vec());
        }
        if requested.len() > MAX_GRID {
            return Err(refuse(format!(
                "`sizes` takes at most {MAX_GRID} entries, and {} were given",
                requested.len()
            )));
        }

        requested
            .iter()
            .map(|size| {
                size.as_u64()
                    .and_then(|size| u32::try_from(size).ok())
                    .filter(|size| *size > 0)
                    .ok_or_else(|| {
                        refuse(format!("`sizes` contains `{size}`, which is not a size"))
                    })
            })
            .collect()
    }
}

/// Replace the document.
struct WriteSvg;

impl Tool for WriteSvg {
    fn name(&self) -> &'static str {
        "write_svg"
    }

    fn description(&self) -> &'static str {
        "Replace the project's SVG source. Send the whole document, not a \
         fragment: call `get_svg` first and return the complete file with your \
         edit applied. The result is validated before anything is replaced, so \
         a document that does not parse, or that has lost its \
         `<shaipe:project>` metadata, is rejected and the project is left \
         exactly as it was. This changes the project in memory; it does not \
         write to disk."
    }

    fn input_schema(&self) -> Value {
        object(
            &[(
                "source",
                string(
                    "The complete SVG document, from `<svg` to `</svg>`, \
                     including the `<metadata>` block.",
                ),
            )],
            &["source"],
        )
    }

    fn mutates(&self) -> bool {
        true
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let source = required_str(self.name(), input, "source")?;

        let invalid = |error: crate::Error| crate::Error::InvalidSvgFromTool {
            tool: self.name().to_owned(),
            source: Box::new(error),
        };

        // Parsed into a throwaway project *before* anything is replaced. This
        // is the whole tool: a model that emits a truncated document must not
        // be able to leave the workspace holding one. `Project::from_source`
        // already performs every check that matters — well-formed XML, an
        // `<svg>` root, `<shaipe:project>` present, a schema version this
        // build understands — so reusing it means this cannot accept something
        // `Project::open` would reject.
        let candidate = Project::from_source(project.path(), source.to_owned()).map_err(invalid)?;

        // And it must be renderable. A document that parses but whose primary
        // variant points at an element that no longer exists is broken in
        // precisely the way an editing model breaks things, and constructing a
        // renderer is cheap enough to pay for catching it. Not a full render
        // of every variant: seconds, to re-establish what this already has.
        Renderer::new(&candidate, RenderOptions::default()).map_err(invalid)?;

        *project = candidate;

        Ok(ToolOutput::json(json!({
            "bytes": source.len(),
            // Said out loud, every time. An agent that assumes it has saved
            // and has not will report work it did not do.
            "saved": false,
            "note": "The project has changed in memory. Nothing has been written to disk.",
        })))
    }
}

/// Replace one variant's element, without resending the whole document.
struct WriteVariant;

impl Tool for WriteVariant {
    fn name(&self) -> &'static str {
        "write_variant"
    }

    fn description(&self) -> &'static str {
        "Replace one variant's element in the document, without resending the \
         whole project. Send the complete replacement for that element — its \
         opening tag, attributes and closing tag, with the same `id` it \
         already has — from `get_svg`. The result is validated by rendering \
         the variant before anything is replaced, so a fragment that does not \
         parse, that drops the element's `id`, or that leaves the variant \
         unrenderable is rejected and the project is left exactly as it was. \
         This only replaces an already-declared variant — use `write_svg` to \
         add a new one, or to change more than one element at once. This \
         changes the project in memory; it does not write to disk."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "variant",
                    string("Which variant to replace. One of the names from `get_variants`."),
                ),
                (
                    "svg",
                    string(
                        "The complete replacement markup for the variant's \
                         element, from its opening tag to its closing tag, \
                         keeping the same `id`.",
                    ),
                ),
            ],
            &["variant", "svg"],
        )
    }

    fn mutates(&self) -> bool {
        true
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let name = required_str(self.name(), input, "variant")?;
        let svg = required_str(self.name(), input, "svg")?;

        // Resolved before the document is touched, so an unknown name is
        // reported without ever attempting to splice anything.
        let element = project.metadata().resolve_variant(name)?.element.clone();

        let invalid = |error: crate::Error| crate::Error::InvalidSvgFromTool {
            tool: self.name().to_owned(),
            source: Box::new(error),
        };

        let updated =
            document::replace_element(project.source(), name, &element, svg, project.path())
                .map_err(invalid)?;

        let candidate = Project::from_source(project.path(), updated).map_err(invalid)?;

        // The touched variant must still isolate and parse as SVG — this
        // catches a dropped or renamed `id`, and a fragment `usvg` refuses.
        // `Format::Svg` because isolation is what this needs to prove; there
        // is nothing to gain from rasterising it too.
        let probe = RenderSpec {
            format: Format::Svg,
            ..RenderSpec::square(name, name, 64)
        };
        Renderer::new(&candidate, RenderOptions::default())
            .and_then(|renderer| renderer.render(&probe))
            .map_err(invalid)?;

        *project = candidate;

        Ok(ToolOutput::json(json!({
            "variant": name,
            "bytes": svg.len(),
            "saved": false,
            "note": "The project has changed in memory. Nothing has been written to disk.",
        })))
    }
}

/// Set one colour in the palette, by name.
struct SetPaletteColour;

impl Tool for SetPaletteColour {
    fn name(&self) -> &'static str {
        "set_palette_colour"
    }

    fn description(&self) -> &'static str {
        "Set a colour in the project's palette: update an existing entry's \
         value or role by name, or declare a new one if the name is not yet \
         in the palette, in which case `value` is required. Only the fields \
         given are changed — omitting `role` when editing an existing colour \
         leaves its role as it was. Setting `value` also restyles the \
         artwork: any element already bound to this colour with \
         `shaipe:fill=\"name\"` or `shaipe:stroke=\"name\"`, alongside its \
         ordinary `fill`/`stroke`, has that attribute rewritten to the new \
         value. Bind an element yourself with `write_variant` or `write_svg` \
         by adding that attribute next to its `fill`/`stroke`; nothing \
         restyles until an element names a colour this way."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "name",
                    string(
                        "How the project refers to this colour, for example \
                         `accent`. One of the names from `get_palette`, or a \
                         new one to declare.",
                    ),
                ),
                (
                    "value",
                    string(
                        "The colour's new value, as a CSS hex colour such as \
                         `#f05032`. Required when `name` does not already \
                         exist in the palette.",
                    ),
                ),
                (
                    "role",
                    string(
                        "What the colour is for, for example `accent`, \
                         `primary`, `secondary`, `background` or \
                         `foreground`. Omit it to leave an existing colour's \
                         role unchanged.",
                    ),
                ),
            ],
            &["name"],
        )
    }

    fn mutates(&self) -> bool {
        true
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let name = required_str(self.name(), input, "name")?;
        let value = input.get("value").and_then(Value::as_str);
        let role = input.get("role").and_then(Value::as_str);

        let refuse = |reason: String| crate::Error::InvalidToolInput {
            tool: self.name().to_owned(),
            reason,
        };

        if value.is_none() && role.is_none() {
            return Err(refuse("`value` or `role` is required".to_owned()));
        }

        let value = value
            .map(str::parse::<Rgba>)
            .transpose()
            .map_err(|error| refuse(format!("`value`: {error}")))?;

        // Restyled before the palette records the new value, so a failure
        // here — unreachable in practice, see `Project::restyle` — leaves the
        // document and the palette in their old, matching state rather than
        // updating one and not the other.
        if let Some(value) = value {
            project.restyle(name, value)?;
        }

        let palette = &mut project.metadata_mut().palette;
        let created = match palette.get_mut(name) {
            Some(colour) => {
                if let Some(value) = value {
                    colour.value = value;
                }
                if let Some(role) = role {
                    colour.role = Some(Role::from(role));
                }
                false
            }
            None => {
                let Some(value) = value else {
                    return Err(refuse(format!(
                        "`{name}` is not yet in the palette, and needs a `value` to declare it"
                    )));
                };
                palette.push(Colour {
                    name: name.to_owned(),
                    value,
                    role: role.map(Role::from),
                });
                true
            }
        };

        let colour = project
            .metadata()
            .palette
            .get(name)
            .expect("just inserted or already present");

        Ok(ToolOutput::json(json!({
            "name": colour.name,
            "value": colour.value.to_string(),
            "role": colour.role.as_ref().map(ToString::to_string),
            "created": created,
        })))
    }
}

/// Attach a file for context, or update one already attached.
struct SetReference;

impl Tool for SetReference {
    fn name(&self) -> &'static str {
        "set_reference"
    }

    fn description(&self) -> &'static str {
        "Attach a file to the project for context, or update one already \
         attached by matching `src` exactly. A new reference defaults to \
         `inspiration` when `kind` is not given. Only the fields given are \
         changed on an existing reference — give an empty `note` to clear \
         it. The file does not need to exist yet; the response's `present` \
         says whether it currently does."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "src",
                    string(
                        "Where the file lives, relative to the project or \
                         absolute. One of the paths from `get_references`, \
                         matched exactly, or a new one to attach.",
                    ),
                ),
                (
                    "kind",
                    string(
                        "Why it is attached: `source` (the thing being \
                         reproduced), `inspiration` (cues, not to copy), \
                         `baseline` (a rendering of this project, for \
                         comparison), or any other label. Defaults to \
                         `inspiration` when attaching a new file; omit it to \
                         leave an existing reference's kind unchanged.",
                    ),
                ),
                (
                    "note",
                    string(
                        "What the file is, in your own words. Omit it to \
                         leave an existing note unchanged; give an empty \
                         string to clear it.",
                    ),
                ),
            ],
            &["src"],
        )
    }

    fn mutates(&self) -> bool {
        true
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let src = required_str(self.name(), input, "src")?;
        let src = PathBuf::from(src);
        let kind = input.get("kind").and_then(Value::as_str);
        let note = input.get("note").and_then(Value::as_str);

        let references = &mut project.metadata_mut().references;
        let created = match references.iter_mut().find(|reference| reference.src == src) {
            Some(reference) => {
                if let Some(kind) = kind {
                    reference.kind = ReferenceKind::from(kind);
                }
                if let Some(note) = note {
                    reference.note = (!note.is_empty()).then(|| note.to_owned());
                }
                false
            }
            None => {
                references.push(Reference {
                    src: src.clone(),
                    kind: kind
                        .map(ReferenceKind::from)
                        .unwrap_or(ReferenceKind::Inspiration),
                    note: note.filter(|note| !note.is_empty()).map(str::to_owned),
                });
                true
            }
        };

        let reference = project
            .metadata()
            .references
            .iter()
            .find(|reference| reference.src == src)
            .expect("just inserted or already present");
        let resolved = project.resolve(&reference.src);

        Ok(ToolOutput::json(json!({
            "src": reference.src.display().to_string(),
            "resolved": resolved.display().to_string(),
            "kind": reference.kind.to_string(),
            "note": reference.note,
            "present": resolved.exists(),
            "created": created,
        })))
    }
}

/// Record how the project's current state was produced.
struct SetGeneration;

impl Tool for SetGeneration {
    fn name(&self) -> &'static str {
        "set_generation"
    }

    fn description(&self) -> &'static str {
        "Record how the project's current state was produced: the agent, the \
         model, and when, as an RFC 3339 timestamp such as \
         `2026-08-23T10:00:00Z`. Call this after you have actually changed \
         the artwork, describing what happened rather than what is about to. \
         Only the fields given are changed; the others keep whatever was \
         recorded before. Nothing here is invented by Shaipe — if you do not \
         know one of these, leave it out rather than guessing."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "agent",
                    string(
                        "The agent that produced the current state, for \
                         example `opencode`.",
                    ),
                ),
                (
                    "model",
                    string("The model it used, for example `claude-sonnet-5`."),
                ),
                (
                    "at",
                    string(
                        "When, as an RFC 3339 timestamp, for example \
                         `2026-08-23T10:00:00Z`.",
                    ),
                ),
            ],
            &[],
        )
    }

    fn mutates(&self) -> bool {
        true
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let agent = input.get("agent").and_then(Value::as_str);
        let model = input.get("model").and_then(Value::as_str);
        let at = input.get("at").and_then(Value::as_str);

        if agent.is_none() && model.is_none() && at.is_none() {
            return Err(crate::Error::InvalidToolInput {
                tool: self.name().to_owned(),
                reason: "at least one of `agent`, `model` or `at` is required".to_owned(),
            });
        }

        let generation = &mut project.metadata_mut().generation;
        if let Some(agent) = agent {
            generation.agent = Some(agent.to_owned());
        }
        if let Some(model) = model {
            generation.model = Some(model.to_owned());
        }
        if let Some(at) = at {
            generation.at = Some(at.to_owned());
        }

        Ok(ToolOutput::json(json!({
            "agent": generation.agent,
            "model": generation.model,
            "at": generation.at,
        })))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::Error;
    use crate::fixtures;
    use crate::tools::Registry;

    fn call(name: &str, input: Value) -> Result<ToolOutput> {
        Registry::new().call(name, &mut fixtures::project(), &input)
    }

    /// The structured half of an answer, for the tools that have no images.
    fn value(name: &str, input: Value) -> Value {
        call(name, input).unwrap().value
    }

    #[test]
    fn get_project_returns_the_same_report_the_cli_prints() {
        // One description of a project, not two that can disagree.
        let value = value("get_project", Value::Null);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["variants"][1]["element"], "mark-wide");
    }

    #[test]
    fn get_variants_says_which_variant_the_document_root_draws() {
        let value = value("get_variants", Value::Null);
        assert_eq!(value[0]["name"], "icon");
        assert_eq!(value[0]["primary"], true);
        assert_eq!(value[1]["primary"], false);
    }

    #[test]
    fn get_palette_returns_every_colour_with_its_role() {
        let value = value("get_palette", Value::Null);
        assert_eq!(value[0]["value"], "#f05032");
        assert_eq!(value[0]["role"], "accent");
        assert_eq!(value[1]["role"], Value::Null);
    }

    #[test]
    fn get_references_lists_every_attached_reference_with_its_presence() {
        // A real file next to a real project, and a reference that points at
        // nothing — `present` has to tell the two apart.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), b"not a real png").unwrap();

        let mut project = Project::open(&path).unwrap();
        project.metadata_mut().references.push(Reference {
            src: PathBuf::from("mockup.png"),
            kind: ReferenceKind::Source,
            note: Some("hand sketch".to_owned()),
        });
        project
            .metadata_mut()
            .references
            .push(Reference::new("missing.png", ReferenceKind::Inspiration));

        let value = Registry::new()
            .call("get_references", &mut project, &Value::Null)
            .unwrap()
            .value;

        assert_eq!(value[0]["src"], "mockup.png");
        assert_eq!(value[0]["kind"], "source");
        assert_eq!(value[0]["note"], "hand sketch");
        assert_eq!(value[0]["present"], true);

        assert_eq!(value[1]["src"], "missing.png");
        assert_eq!(value[1]["kind"], "inspiration");
        assert_eq!(value[1]["note"], Value::Null);
        assert_eq!(value[1]["present"], false);
    }

    #[test]
    fn get_reference_image_returns_the_attached_files_bytes_and_mime_type() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        let bytes = b"not a real png, but bytes all the same".to_vec();
        std::fs::write(directory.path().join("mockup.png"), &bytes).unwrap();

        let mut project = Project::open(&path).unwrap();
        project.metadata_mut().references.push(Reference {
            src: PathBuf::from("mockup.png"),
            kind: ReferenceKind::Source,
            note: Some("hand sketch".to_owned()),
        });

        let output = Registry::new()
            .call(
                "get_reference_image",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap();

        assert_eq!(output.value["src"], "mockup.png");
        assert_eq!(output.value["kind"], "source");
        assert_eq!(output.value["note"], "hand sketch");
        assert_eq!(output.value["mime_type"], "image/png");

        // The image travels beside the JSON, not inside it, exactly like
        // `render_svg`'s.
        let [image] = &output.images[..] else {
            panic!("expected exactly one image, got {}", output.images.len());
        };
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.bytes, bytes);
        assert_eq!(image.label, "mockup.png (source)");
    }

    #[test]
    fn get_reference_image_of_an_unknown_src_lists_the_ones_that_are_attached() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_image",
                &mut project,
                &json!({ "src": "watermark.png" }),
            )
            .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("mockup.png"), "{rendered}");
        assert!(matches!(error, Error::InvalidToolInput { .. }));
    }

    #[test]
    fn get_reference_image_of_a_missing_file_reports_why() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("missing.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_image",
                &mut project,
                &json!({ "src": "missing.png" }),
            )
            .unwrap_err();
        assert!(matches!(error, Error::Io { .. }));
    }

    #[test]
    fn get_reference_image_rejects_an_extension_it_does_not_recognise() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.pdf"), b"not an image").unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.pdf", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_image",
                &mut project,
                &json!({ "src": "mockup.pdf" }),
            )
            .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("mockup.pdf"), "{rendered}");
        assert!(matches!(error, Error::InvalidToolInput { .. }));
    }

    /// A tiny synthetic PNG: a black ring on a transparent background,
    /// `size` pixels square — same shape [`crate::vectorize`]'s own tests
    /// build, reused here so this module does not need its own fixture file.
    fn ring_png(size: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::new(size, size);
        let center = f64::from(size) / 2.0;
        let outer = center - 4.0;
        let inner = outer / 2.5;
        for y in 0..size {
            for x in 0..size {
                let dx = f64::from(x) - center;
                let dy = f64::from(y) - center;
                let d = (dx * dx + dy * dy).sqrt();
                let pixel = if d <= outer && d >= inner {
                    image::Rgba([0, 0, 0, 255])
                } else {
                    image::Rgba([0, 0, 0, 0])
                };
                img.put_pixel(x, y, pixel);
            }
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    #[test]
    fn get_reference_trace_returns_svg_markup_with_no_colour_baked_in() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), ring_png(64)).unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let output = Registry::new()
            .call(
                "get_reference_trace",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap();

        assert_eq!(output.value["src"], "mockup.png");
        assert_eq!(output.value["path_count"], 1);
        let svg = output.value["svg"].as_str().unwrap();
        assert!(svg.contains("<path"), "{svg}");
        // No mutation: this is a `get_` tool, and the project is unread by
        // anything other than resolving where the reference lives.
        assert!(
            !Registry::new()
                .get("get_reference_trace")
                .unwrap()
                .mutates()
        );
    }

    #[test]
    fn get_reference_trace_of_an_unknown_src_lists_the_ones_that_are_attached() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_trace",
                &mut project,
                &json!({ "src": "watermark.png" }),
            )
            .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("mockup.png"), "{rendered}");
        assert!(matches!(error, Error::InvalidToolInput { .. }));
    }

    #[test]
    fn get_reference_trace_of_bytes_that_are_not_an_image_reports_why() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), b"not a real png").unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_trace",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::Decode { .. }));
    }

    #[test]
    fn get_reference_trace_of_a_blank_image_says_nothing_traced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        let blank = image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 255, 255, 255]));
        let mut bytes = Vec::new();
        blank
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        std::fs::write(directory.path().join("mockup.png"), bytes).unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_trace",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::EmptyTrace { .. }));
    }

    #[test]
    fn get_reference_trace_threshold_and_invert_are_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), ring_png(64)).unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let output = Registry::new()
            .call(
                "get_reference_trace",
                &mut project,
                &json!({ "src": "mockup.png", "threshold": 200, "invert": false }),
            )
            .unwrap();

        assert!(output.value["svg"].as_str().unwrap().contains("<path"));
    }

    #[test]
    fn render_svg_returns_a_decodable_png_of_the_requested_size() {
        let output = call("render_svg", json!({ "variant": "icon", "width": 64 })).unwrap();

        assert_eq!(output.value["width"], 64);
        assert_eq!(output.value["height"], 64);

        // The image travels beside the JSON, not inside it. A model that gets
        // base64 in a string field sees a string.
        let [image] = &output.images[..] else {
            panic!("expected exactly one image, got {}", output.images.len());
        };
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.label, "icon at 64x64");

        let pixmap = tiny_skia::Pixmap::decode_png(&image.bytes).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (64, 64));
    }

    #[test]
    fn render_svg_defaults_to_a_square_so_a_model_need_not_invent_a_size() {
        let value = value("render_svg", json!({ "variant": "icon" }));
        assert_eq!(value["width"], 512);
        assert_eq!(value["height"], 512);
    }

    #[test]
    fn render_svg_without_a_variant_says_which_argument_is_missing() {
        let error = call("render_svg", json!({ "width": 64 })).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("variant"), "{rendered}");
    }

    #[test]
    fn render_svg_of_an_undeclared_variant_lists_the_ones_that_exist() {
        // The recovery path for a model that guessed a name: the error tells
        // it what it should have called instead.
        let error = call("render_svg", json!({ "variant": "watermark" })).unwrap_err();
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("icon"), "{rendered}");
        assert!(rendered.contains("wordmark"), "{rendered}");
    }

    #[test]
    fn only_the_writing_and_setting_tools_declare_that_they_mutate() {
        // `get_` and `render_` promise no mutation in ADR 007, and
        // `Tool::mutates` says the same thing in code. This is what stops the
        // two from disagreeing.
        let mutating: Vec<_> = Registry::new()
            .tools()
            .filter(|tool| tool.mutates())
            .map(Tool::name)
            .collect();
        assert_eq!(
            mutating,
            [
                "set_generation",
                "set_palette_colour",
                "set_reference",
                "write_svg",
                "write_variant"
            ]
        );
    }

    #[test]
    fn get_svg_includes_a_prompt_edit_that_has_not_been_saved() {
        // `Project` keeps the bytes and the parsed metadata side by side, and
        // only saving reconciles them. Serving the bytes showed the agent a
        // document that disagreed with `get_project` in the same turn.
        let mut project = fixtures::project();
        project.metadata_mut().prompt = Some("a wordless circular mark".to_owned());

        let value = Registry::new()
            .call("get_svg", &mut project, &Value::Null)
            .unwrap()
            .value;

        let source = value["source"].as_str().unwrap();
        assert!(source.contains("a wordless circular mark"), "{source}");
        assert_eq!(value["bytes"], source.len());
    }

    #[test]
    fn get_svg_and_get_project_agree_about_the_prompt() {
        let mut project = fixtures::project();
        project.metadata_mut().prompt = Some("a wordless circular mark".to_owned());

        let registry = Registry::new();
        let described = registry
            .call("get_project", &mut project, &Value::Null)
            .unwrap()
            .value;
        let document = registry
            .call("get_svg", &mut project, &Value::Null)
            .unwrap()
            .value;

        let prompt = described["prompt"].as_str().unwrap();
        assert!(document["source"].as_str().unwrap().contains(prompt));
    }

    #[test]
    fn a_write_that_round_trips_get_svg_preserves_an_unsaved_prompt_edit() {
        // The whole failure, end to end: someone edits the prompt, the agent
        // reads the document, changes a colour and writes it back. Before this
        // the write reverted the prompt without a word, and the workspace then
        // pulled the reverted text back into the pane.
        let mut project = fixtures::project();
        project.metadata_mut().prompt = Some("a wordless circular mark".to_owned());

        let registry = Registry::new();
        let document = registry
            .call("get_svg", &mut project, &Value::Null)
            .unwrap()
            .value;

        // What a model does: edit the document it was handed.
        let edited = document["source"]
            .as_str()
            .unwrap()
            .replace("#f05032", "#0066ff");

        registry
            .call("write_svg", &mut project, &json!({ "source": edited }))
            .unwrap();

        assert_eq!(
            project.metadata().prompt.as_deref(),
            Some("a wordless circular mark"),
            "the agent's write reverted the prompt"
        );
        assert_eq!(
            project.metadata().palette.colours()[0].value.to_string(),
            "#0066ff",
            "the agent's own edit was lost"
        );
    }

    #[test]
    fn get_svg_is_byte_identical_to_the_file_for_an_untouched_project() {
        // The other side of serving `to_svg()`: it must not reformat a project
        // nobody has edited, or every agent read would look like a diff.
        let value = value("get_svg", Value::Null);
        assert_eq!(value["source"].as_str().unwrap(), fixtures::PROJECT);
    }

    #[test]
    fn render_grid_returns_one_image_per_size_at_its_true_resolution() {
        // The point of the tool. A contact sheet would upscale the small
        // sizes, which is a lie about the exact thing being checked.
        let output = call(
            "render_grid",
            json!({ "variant": "icon", "sizes": [64, 16] }),
        )
        .unwrap();

        assert_eq!(output.images.len(), 2);
        for (image, expected) in output.images.iter().zip([64, 16]) {
            let pixmap = tiny_skia::Pixmap::decode_png(&image.bytes).unwrap();
            assert_eq!(
                (pixmap.width(), pixmap.height()),
                (expected, expected),
                "{} was not rendered at its own size",
                image.label
            );
        }
    }

    #[test]
    fn render_grid_defaults_to_sizes_that_show_where_legibility_fails() {
        let output = call("render_grid", json!({ "variant": "icon" })).unwrap();
        assert_eq!(output.value["sizes"], json!([512, 128, 64, 32, 16]));
        assert_eq!(output.images.len(), 5);
    }

    #[test]
    fn render_grid_refuses_an_unbounded_list_of_sizes() {
        // An unbounded list is an unbounded quantity of image in one response.
        let error = call(
            "render_grid",
            json!({ "variant": "icon", "sizes": [1, 2, 3, 4, 5, 6, 7, 8, 9] }),
        )
        .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("at most 8"), "{rendered}");
    }

    #[test]
    fn a_background_that_is_not_a_colour_names_the_argument_it_came_from() {
        // The colour parser knows the value is bad but not which argument
        // carried it, and a model needs both to correct itself.
        let error = call(
            "render_svg",
            json!({ "variant": "icon", "background": "octarine" }),
        )
        .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("background"), "{rendered}");
        assert!(rendered.contains("octarine"), "{rendered}");
    }

    #[test]
    fn write_svg_accepts_a_valid_edit_and_the_project_reflects_it() {
        let mut project = fixtures::project();
        let edited = fixtures::PROJECT.replace("#f05032", "#00ff00");

        Registry::new()
            .call("write_svg", &mut project, &json!({ "source": edited }))
            .unwrap();

        assert_eq!(
            project.source(),
            fixtures::PROJECT.replace("#f05032", "#00ff00")
        );
        assert_eq!(
            project.metadata().palette.colours()[0].value.to_string(),
            "#00ff00"
        );
    }

    #[test]
    fn write_svg_rejects_a_document_that_is_not_well_formed_and_changes_nothing() {
        let mut project = fixtures::project();
        let error = Registry::new()
            .call(
                "write_svg",
                &mut project,
                &json!({ "source": "<svg><circle r=\"5\"" }),
            )
            .unwrap_err();

        assert!(matches!(error, crate::Error::InvalidSvgFromTool { .. }));
        // The half of the contract the description promises.
        assert_eq!(project.source(), fixtures::PROJECT);
    }

    #[test]
    fn write_svg_rejects_a_document_that_has_lost_its_metadata() {
        // The realistic failure: a model rewrites the artwork and drops the
        // `<metadata>` block it did not think was part of the picture. Left
        // unchecked, that silently destroys the palette, the variants and the
        // render specifications.
        let mut project = fixtures::project();
        let error = Registry::new()
            .call(
                "write_svg",
                &mut project,
                &json!({
                    "source": "<svg xmlns=\"http://www.w3.org/2000/svg\" \
                               viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"30\"/></svg>"
                }),
            )
            .unwrap_err();

        assert!(matches!(error, crate::Error::InvalidSvgFromTool { .. }));
        assert_eq!(project.source(), fixtures::PROJECT);

        // And the diagnostic has to tell it how to succeed next time.
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("get_svg"), "{rendered}");
        assert!(rendered.contains("metadata"), "{rendered}");
    }

    #[test]
    fn write_svg_does_not_touch_the_file_on_disk() {
        // Invariant 2: Shaipe never dirties a working tree unasked. An agent
        // silently rewriting a tracked file is how a tool stops being trusted.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();

        let mut project = Project::open(&path).unwrap();
        Registry::new()
            .call(
                "write_svg",
                &mut project,
                &json!({ "source": fixtures::PROJECT.replace("#f05032", "#00ff00") }),
            )
            .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), fixtures::PROJECT);
    }

    #[test]
    fn write_svg_says_it_has_not_saved() {
        // In the response, not only in the description: an agent that assumes
        // it has saved and has not will report work it did not do.
        let mut project = fixtures::project();
        let output = Registry::new()
            .call(
                "write_svg",
                &mut project,
                &json!({ "source": fixtures::PROJECT }),
            )
            .unwrap();

        assert_eq!(output.value["saved"], false);
    }

    #[test]
    fn write_variant_replaces_only_the_named_elements_bytes() {
        let mut project = fixtures::project();

        let output = Registry::new()
            .call(
                "write_variant",
                &mut project,
                &json!({
                    "variant": "icon",
                    "svg": r##"<symbol id="icon" viewBox="0 0 64 64"><circle r="32" cx="32" cy="32" fill="#f05032"/></symbol>"##,
                }),
            )
            .unwrap();

        assert_eq!(output.value["variant"], "icon");
        assert_eq!(output.value["saved"], false);

        assert!(
            project
                .source()
                .contains(r#"<circle r="32" cx="32" cy="32""#)
        );
        assert!(!project.source().contains(r#"<rect width="64" height="64""#));
        // The other variant, and the metadata, are untouched.
        assert!(project.source().contains(r#"id="mark-wide""#));
        assert!(project.source().contains("A square and a bar."));
    }

    #[test]
    fn write_variant_of_an_undeclared_variant_lists_the_ones_that_exist() {
        let error = call(
            "write_variant",
            json!({ "variant": "watermark", "svg": "<g/>" }),
        )
        .unwrap_err();

        assert!(matches!(error, crate::Error::UnknownVariant { .. }));
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("icon"), "{rendered}");
        assert!(rendered.contains("wordmark"), "{rendered}");
    }

    #[test]
    fn write_variant_rejects_a_fragment_that_is_not_well_formed_and_changes_nothing() {
        let mut project = fixtures::project();
        let error = Registry::new()
            .call(
                "write_variant",
                &mut project,
                &json!({ "variant": "icon", "svg": "<symbol id=\"icon\"" }),
            )
            .unwrap_err();

        assert!(matches!(error, crate::Error::InvalidSvgFromTool { .. }));
        assert_eq!(project.source(), fixtures::PROJECT);
    }

    #[test]
    fn write_variant_rejects_a_replacement_that_drops_the_elements_id() {
        // The realistic failure: a model rewrites the geometry and loses the
        // `id` the variant's metadata still points at. Left unchecked, the
        // variant would silently stop rendering.
        let mut project = fixtures::project();
        let error = Registry::new()
            .call(
                "write_variant",
                &mut project,
                &json!({
                    "variant": "icon",
                    "svg": r#"<symbol viewBox="0 0 64 64"><rect width="64" height="64"/></symbol>"#,
                }),
            )
            .unwrap_err();

        assert!(matches!(error, crate::Error::InvalidSvgFromTool { .. }));
        assert_eq!(project.source(), fixtures::PROJECT);
    }

    #[test]
    fn write_variant_does_not_touch_the_file_on_disk() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();

        let mut project = Project::open(&path).unwrap();
        Registry::new()
            .call(
                "write_variant",
                &mut project,
                &json!({
                    "variant": "icon",
                    "svg": r#"<symbol id="icon" viewBox="0 0 64 64"><circle r="32" cx="32" cy="32"/></symbol>"#,
                }),
            )
            .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), fixtures::PROJECT);
    }

    #[test]
    fn set_palette_colour_updates_an_existing_colours_value_and_keeps_its_role() {
        let mut project = fixtures::project();

        let output = Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "accent", "value": "#0066ff" }),
            )
            .unwrap();

        assert_eq!(output.value["created"], false);
        assert_eq!(output.value["value"], "#0066ff");
        assert_eq!(output.value["role"], "accent");

        let colour = project.metadata().palette.get("accent").unwrap();
        assert_eq!(colour.value.to_string(), "#0066ff");
        assert_eq!(colour.role.as_ref().unwrap().to_string(), "accent");
    }

    #[test]
    fn set_palette_colour_can_set_only_the_role_leaving_the_value_alone() {
        let mut project = fixtures::project();

        Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "ink", "role": "foreground" }),
            )
            .unwrap();

        let colour = project.metadata().palette.get("ink").unwrap();
        assert_eq!(colour.value.to_string(), "#18181b");
        assert_eq!(colour.role.as_ref().unwrap().to_string(), "foreground");
    }

    #[test]
    fn set_palette_colour_declares_a_new_colour_when_the_name_is_unknown() {
        let mut project = fixtures::project();

        let output = Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "highlight", "value": "#ffcc00", "role": "secondary" }),
            )
            .unwrap();

        assert_eq!(output.value["created"], true);
        assert_eq!(project.metadata().palette.len(), 3);
        let colour = project.metadata().palette.get("highlight").unwrap();
        assert_eq!(colour.value.to_string(), "#ffcc00");
        assert_eq!(colour.role.as_ref().unwrap().to_string(), "secondary");
    }

    #[test]
    fn set_palette_colour_refuses_a_new_name_without_a_value() {
        let error = call(
            "set_palette_colour",
            json!({ "name": "highlight", "role": "secondary" }),
        )
        .unwrap_err();

        assert!(matches!(error, crate::Error::InvalidToolInput { .. }));
        let rendered = error.to_string();
        assert!(rendered.contains("highlight"), "{rendered}");
    }

    #[test]
    fn set_palette_colour_refuses_neither_value_nor_role() {
        let error = call("set_palette_colour", json!({ "name": "accent" })).unwrap_err();

        assert!(matches!(error, crate::Error::InvalidToolInput { .. }));
        let rendered = error.to_string();
        assert!(rendered.contains("value"), "{rendered}");
        assert!(rendered.contains("role"), "{rendered}");
    }

    #[test]
    fn set_palette_colour_names_the_argument_a_bad_value_came_from() {
        let error = call(
            "set_palette_colour",
            json!({ "name": "accent", "value": "octarine" }),
        )
        .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("value"), "{rendered}");
        assert!(rendered.contains("octarine"), "{rendered}");
    }

    /// A project with one element bound to `accent` via `shaipe:fill`,
    /// alongside its literal `fill` — the artwork half of the fixture
    /// `set_palette_colour`'s restyling tests need, which the shared
    /// [`fixtures::PROJECT`] does not declare any binding for.
    const BOUND_PROJECT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 64 64">
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

    #[test]
    fn set_palette_colour_restyles_every_element_bound_to_it() {
        let mut project = Project::from_source("logo.svg", BOUND_PROJECT.to_owned()).unwrap();

        Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "accent", "value": "#0066ff" }),
            )
            .unwrap();

        assert!(project.source().contains(r##"fill="#0066ff""##));
        assert!(!project.source().contains(r##"fill="#f05032""##));
    }

    #[test]
    fn set_palette_colour_setting_only_the_role_does_not_touch_bound_artwork() {
        let mut project = Project::from_source("logo.svg", BOUND_PROJECT.to_owned()).unwrap();

        Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "accent", "role": "primary" }),
            )
            .unwrap();

        assert!(project.source().contains(r##"fill="#f05032""##));
    }

    #[test]
    fn set_reference_attaches_a_new_file_defaulting_to_inspiration() {
        let mut project = fixtures::project();

        let output = Registry::new()
            .call(
                "set_reference",
                &mut project,
                &json!({ "src": "mockup.png", "note": "hand sketch" }),
            )
            .unwrap();

        assert_eq!(output.value["created"], true);
        assert_eq!(output.value["kind"], "inspiration");
        assert_eq!(output.value["note"], "hand sketch");
        assert_eq!(output.value["present"], false);

        let reference = project
            .metadata()
            .references
            .iter()
            .find(|reference| reference.src == Path::new("mockup.png"))
            .unwrap();
        assert_eq!(reference.kind, ReferenceKind::Inspiration);
        assert_eq!(reference.note.as_deref(), Some("hand sketch"));
    }

    #[test]
    fn set_reference_updates_an_existing_references_kind_and_keeps_its_note() {
        let mut project = fixtures::project();
        project.metadata_mut().references.push(Reference {
            src: PathBuf::from("mockup.png"),
            kind: ReferenceKind::Inspiration,
            note: Some("hand sketch".to_owned()),
        });

        let output = Registry::new()
            .call(
                "set_reference",
                &mut project,
                &json!({ "src": "mockup.png", "kind": "source" }),
            )
            .unwrap();

        assert_eq!(output.value["created"], false);
        assert_eq!(output.value["kind"], "source");
        // Not given, so unchanged.
        assert_eq!(output.value["note"], "hand sketch");
    }

    #[test]
    fn set_reference_can_clear_an_existing_note_with_an_empty_string() {
        let mut project = fixtures::project();
        project.metadata_mut().references.push(Reference {
            src: PathBuf::from("mockup.png"),
            kind: ReferenceKind::Inspiration,
            note: Some("hand sketch".to_owned()),
        });

        Registry::new()
            .call(
                "set_reference",
                &mut project,
                &json!({ "src": "mockup.png", "note": "" }),
            )
            .unwrap();

        let reference = project
            .metadata()
            .references
            .iter()
            .find(|reference| reference.src == Path::new("mockup.png"))
            .unwrap();
        assert_eq!(reference.note, None);
    }

    #[test]
    fn set_reference_calling_it_again_for_the_same_src_with_nothing_else_is_a_no_op() {
        let mut project = fixtures::project();
        project.metadata_mut().references.push(Reference {
            src: PathBuf::from("mockup.png"),
            kind: ReferenceKind::Source,
            note: Some("hand sketch".to_owned()),
        });

        let output = Registry::new()
            .call(
                "set_reference",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap();

        assert_eq!(output.value["created"], false);
        assert_eq!(output.value["kind"], "source");
        assert_eq!(output.value["note"], "hand sketch");
        assert_eq!(project.metadata().references.len(), 1);
    }

    #[test]
    fn set_reference_without_a_src_says_which_argument_is_missing() {
        let error = call("set_reference", json!({ "kind": "source" })).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("src"), "{rendered}");
    }

    #[test]
    fn set_generation_records_every_field_given() {
        let mut project = fixtures::project();

        let output = Registry::new()
            .call(
                "set_generation",
                &mut project,
                &json!({ "agent": "opencode", "model": "claude-sonnet-5", "at": "2026-08-23T10:00:00Z" }),
            )
            .unwrap();

        assert_eq!(output.value["agent"], "opencode");
        let generation = &project.metadata().generation;
        assert_eq!(generation.agent.as_deref(), Some("opencode"));
        assert_eq!(generation.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(generation.at.as_deref(), Some("2026-08-23T10:00:00Z"));
    }

    #[test]
    fn set_generation_patches_a_single_field_and_leaves_the_others_recorded() {
        let mut project = fixtures::project();
        project.metadata_mut().generation = crate::project::Generation {
            agent: Some("opencode".to_owned()),
            model: Some("claude-sonnet-5".to_owned()),
            at: Some("2026-08-23T10:00:00Z".to_owned()),
        };

        Registry::new()
            .call(
                "set_generation",
                &mut project,
                &json!({ "at": "2026-08-23T11:30:00Z" }),
            )
            .unwrap();

        let generation = &project.metadata().generation;
        assert_eq!(generation.agent.as_deref(), Some("opencode"));
        assert_eq!(generation.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(generation.at.as_deref(), Some("2026-08-23T11:30:00Z"));
    }

    #[test]
    fn set_generation_refuses_a_call_with_no_fields_at_all() {
        let error = call("set_generation", json!({})).unwrap_err();
        assert!(matches!(error, crate::Error::InvalidToolInput { .. }));
    }
}
