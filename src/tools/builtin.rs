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
        Box::new(GetReferenceAnalysis),
        Box::new(GetReferenceImage),
        Box::new(GetReferenceTrace),
        Box::new(CompareReference),
        Box::new(GetWorkflow),
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

/// What every write tool says about the artwork it has just accepted.
///
/// Acceptance means the document is valid, not that it looks right, and an
/// agent that has just been told `ok` is inclined to stop. The pointer to the
/// next step travels in the result rather than only in the description, which
/// the model read many turns ago.
const AFTER_A_WRITE: &str = "Valid is not finished: call `render_svg` to look at the result. \
    For a `source` reference, call `compare_reference` as well and fix the largest difference.";

/// What every write tool says about where the change went.
///
/// Says what *this call* did rather than what the project is: under
/// `shaipe mcp --write` the session saves after a mutating call, so a flat
/// "nothing was written" would be false there.
const IN_MEMORY: &str = "The project has changed in memory. This call did not write to disk; \
    that happens only if the server was started with `--write` or the user saves.";

/// The attached reference a `src` names, or the refusal that lists the ones
/// there are.
///
/// One place rather than one copy per reference tool: five tools resolve a
/// `src` and the wording that tells a model how to recover should not differ
/// between them. Matched exactly against what is recorded, never against the
/// resolved path, so the only spelling that works is the one `get_references`
/// prints.
fn attached_reference(tool: &str, project: &Project, src: &Path) -> Result<Reference> {
    let references = &project.metadata().references;

    references
        .iter()
        .find(|reference| reference.src == src)
        .cloned()
        .ok_or_else(|| crate::Error::InvalidToolInput {
            tool: tool.to_owned(),
            reason: format!(
                "`{}` is not an attached reference. {}",
                src.display(),
                if references.is_empty() {
                    "Nothing is attached yet; call `set_reference` to attach one.".to_owned()
                } else {
                    format!(
                        "Attached: {}. Pass one of these exactly as `get_references` lists it, \
                         or attach a new file with `set_reference`.",
                        references
                            .iter()
                            .map(|reference| reference.src.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            ),
        })
}

/// Turn a colour that would not parse into a refusal that says how to fix it.
///
/// The parse error carries the accepted formats as its help, and re-wrapping
/// it as an `InvalidToolInput` by `Display` alone used to throw that away: the
/// model was told its colour was wrong and not what a right one looks like.
fn bad_colour(tool: &str, argument: &str, error: &crate::Error) -> crate::Error {
    use miette::Diagnostic as _;

    crate::Error::InvalidToolInput {
        tool: tool.to_owned(),
        reason: match error.help() {
            Some(help) => format!("`{argument}`: {error}. {help}"),
            None => format!("`{argument}`: {error}"),
        },
    }
}

/// Describe the whole project.
struct GetProject;

impl Tool for GetProject {
    fn name(&self) -> &'static str {
        "get_project"
    }

    fn description(&self) -> &'static str {
        "Describe the whole project in one call: its prompt, palette, fonts, \
         variants, references and the assets it declares. A good first call, \
         to learn what the project contains and what its variants and colours \
         are named; `get_variants`, `get_palette` and `get_references` return \
         the same parts on their own."
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
         id in the document that draws each one. The names are what \
         `render_svg`, `compare_reference` and `write_variant` take; the \
         element id is what a rewritten element must keep. `primary` marks \
         the variant the document root draws."
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
         editing the artwork, and change one with `set_palette_colour`."
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
         documents recorded as a `source` to reproduce, `inspiration` to take \
         cues from, or a `baseline` rendering to compare against. Each entry \
         reports whether the file exists (`present`) but not what it shows: \
         call `get_reference_image` to look at one. The reference tools take \
         a `src` exactly as listed here, and only for a present raster image; \
         `set_reference` attaches another."
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

/// Measure an attached reference's pixels into objective facts, algorithmically.
///
/// See [`crate::analysis`] and ADR 018 for why this exists: dimensions,
/// dominant colours, region bounding boxes and centroids are measurements, not
/// descriptions, and a vision model reading pixels cannot report them any
/// more precisely than it can report an exact curve — the same gap
/// `get_reference_trace`/ADR 014 closes for geometry.
struct GetReferenceAnalysis;

impl Tool for GetReferenceAnalysis {
    fn name(&self) -> &'static str {
        "get_reference_analysis"
    }

    fn description(&self) -> &'static str {
        "Measure an attached raster reference's pixels into objective facts: \
         dimensions, a background/foreground split, dominant colours, \
         connected regions with bounding boxes and centroids, holes, and \
         left-right/top-bottom symmetry scores, all in the reference's own \
         pixels. `appearance` says, per region, whether its fill is flat, a \
         linear or radial gradient (axis or centre, stops with opacity) or \
         varied (not vouched for as a gradient), whether it looks like a \
         stroke, and how transparent the image is. This is measurement, not interpretation — it cannot say a \
         region is a \"castle\" or a \"letter A\", only where it is, how big \
         and what colour; naming it remains your job. Call it after \
         `get_reference_image` and before `get_reference_trace` or \
         constructing a variant by hand, to ground layout and colour in real \
         pixels rather than in a guess. Not for photographs, busy \
         backgrounds, or artwork that touches the canvas edge: it separates \
         only one background colour, sampled from the border, from \
         everything else."
    }

    fn input_schema(&self) -> Value {
        object(
            &[(
                "src",
                string(
                    "Which reference to analyse. One of the paths from \
                     `get_references`, matched exactly.",
                ),
            )],
            &["src"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let src = PathBuf::from(required_str(self.name(), input, "src")?);

        // Reported before anything is read, so an unattached name is a clear
        // refusal, not a race.
        let reference = attached_reference(self.name(), project, &src)?;

        let resolved = project.resolve(&reference.src);

        // Before the file is read, like the other reference tools: an
        // unsupported format is refused for what it is, not for a decode
        // failure that says less.
        reference_mime_type_or_refuse(self.name(), &resolved)?;

        let bytes =
            std::fs::read(&resolved).map_err(|error| crate::Error::io(resolved.clone(), error))?;

        let analysis = crate::analysis::analyze(&resolved, &bytes)?;

        Ok(ToolOutput::json(json!({
            "src": reference.src.display().to_string(),
            "resolved": resolved.display().to_string(),
            "analysis": serde_json::to_value(&analysis)
                .expect("Analysis always serialises"),
        })))
    }
}

/// Read one attached reference's bytes and hand them to whoever is looking.
struct GetReferenceImage;

impl Tool for GetReferenceImage {
    fn name(&self) -> &'static str {
        "get_reference_image"
    }

    fn description(&self) -> &'static str {
        "Look at an attached raster reference (PNG, JPEG, GIF, WEBP or BMP): \
         returns the image itself and, when the header can be read, its pixel \
         dimensions. `get_references` only says a file exists, not what it \
         shows, so look before you measure, trace or write anything. Cannot \
         show an SVG or PDF reference. The image is returned as it is, \
         unresized; its dimensions are the size `compare_reference` will \
         render at."
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

        // Resolved before anything is read, so a name that was never attached
        // is reported without ever touching the filesystem.
        let reference = attached_reference(self.name(), project, &src)?;

        let resolved = project.resolve(&reference.src);

        // Checked before the file is read: an SVG or PDF reference is refused
        // for what it is, rather than for whatever reading it happened to
        // say first. Guessed from the extension, never sniffed from the
        // bytes, so an agent that attaches a mislabelled file gets a
        // mislabelled image — its mistake to notice, not Shaipe's to fix.
        let mime_type = reference_mime_type_or_refuse(self.name(), &resolved)?;

        // Missing or unreadable reads exactly like any other file-access
        // failure in the crate — `get_references`' `present` already told the
        // model whether this was likely, so a surprise here is a race, not a
        // guess.
        let bytes =
            std::fs::read(&resolved).map_err(|error| crate::Error::io(resolved.clone(), error))?;

        Ok(ToolOutput::json(json!({
            "src": reference.src.display().to_string(),
            "resolved": resolved.display().to_string(),
            "kind": reference.kind.to_string(),
            "note": reference.note,
            "mime_type": mime_type,
            // What `compare_reference` will render at, and so what a model
            // should size its own renders to. Null rather than an error for
            // bytes that do not decode: this tool has never refused those, and
            // `get_reference_analysis` is where a bad image is reported.
            "dimensions": pixel_dimensions(&bytes),
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

/// [`reference_mime_type`], or the refusal that lists what is accepted.
///
/// Shared by every tool that reads a reference's pixels, so the message that
/// tells a model what to do instead is one message, and an unusable reference
/// is refused for what it is before any of them touches the file.
fn reference_mime_type_or_refuse(tool: &str, resolved: &Path) -> Result<&'static str> {
    reference_mime_type(resolved).ok_or_else(|| crate::Error::InvalidToolInput {
        tool: tool.to_owned(),
        reason: format!(
            "`{}` is not an image format this tool recognises; expected .png, .jpg, .jpeg, \
             .gif, .webp or .bmp. An SVG or PDF reference cannot be used here; export a raster \
             image of it and attach that with `set_reference`.",
            resolved.display()
        ),
    })
}

/// A raster's width and height from its header, or null when it has none.
///
/// The header only: the image is not decoded, so this costs nothing next to
/// reading the file.
fn pixel_dimensions(bytes: &[u8]) -> Value {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
        .map_or(
            Value::Null,
            |(width, height)| json!({ "width": width, "height": height }),
        )
}

/// Trace an attached reference's pixels into vector paths, algorithmically.
///
/// See [`crate::vectorize`] and ADR 014/020 for why this exists at all: an
/// LLM writing `<path d="...">` from a prompt and an image can name the
/// shapes present but cannot reproduce an exact curve or corner radius from
/// it — that is a measurement, not a description, and this tool is what
/// actually measures.
struct GetReferenceTrace;

impl Tool for GetReferenceTrace {
    fn name(&self) -> &'static str {
        "get_reference_trace"
    }

    fn description(&self) -> &'static str {
        "Trace an attached raster reference into vector paths by measuring \
         its pixels, not by describing its shape: exact curves, corner radii \
         and tapering that cannot be reproduced by eye. Call it after \
         `get_reference_analysis`, and only for a clean, flat-colour mark; \
         construct photographs and busy artwork yourself. `silhouette` \
         (default) separates one foreground colour from one background into \
         a single shape with holes, for a single-colour mark. `colour` \
         traces each region as its own path, for multi-colour artwork; a \
         gradient or translucent fill stays one path painted with its \
         measured fill, and `fallbacks` lists regions that vary but are not \
         one. The background becomes one more region. \
         Returns a standalone SVG in the reference's own pixel space with no \
         `viewBox` — give a grafted element a `viewBox` or a transform — an \
         overall bounding box, up to 32 paths (bounding box, approximate area \
         and fill) with the true path count, and a preview image. The trace \
         is raw material, not the deliverable: graft it into a variant with \
         `write_variant` or `write_svg`, keeping the variant's element `id`, \
         then look with `render_svg`."
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
                    "mode",
                    json!({
                        "type": "string",
                        "enum": ["silhouette", "colour"],
                        "description": "`silhouette` (default): one foreground \
                         colour against one background, as a single shape — \
                         for a single-colour mark. `colour`: flat-colour \
                         regions, each traced separately with its own fill — \
                         choose it when `get_reference_analysis` finds more \
                         than one dominant colour, or when `silhouette` \
                         returns fragments. `threshold` and `invert` only \
                         affect `silhouette`; `max_colors` only affects \
                         `colour`.",
                    }),
                ),
                (
                    "threshold",
                    json!({
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 255,
                        "description": "`silhouette` mode only. Binary cutoff \
                         separating foreground from background: pixels \
                         darker than this are traced. Lower it if background \
                         is being traced instead of the mark; raise it if \
                         part of the mark is missed. Many small fragments \
                         instead of one shape usually mean a mid-brightness \
                         mark (a saturated green or blue is about as bright \
                         as its background): try 200 or more, or switch to \
                         `colour` mode. Omit for the default, 128.",
                    }),
                ),
                (
                    "invert",
                    boolean(
                        "`silhouette` mode only. Set when the reference is \
                         light artwork on a dark background, so light pixels \
                         are traced as the foreground instead of dark ones.",
                    ),
                ),
                (
                    "max_colors",
                    json!({
                        "type": "integer",
                        "minimum": 1,
                        "description": "`colour` mode only. Caps how many \
                         distinct colours the clustering keeps, merging the \
                         rest into their nearest neighbour. Omit to let \
                         vtracer's own clustering decide.",
                    }),
                ),
                (
                    "appearance",
                    boolean(
                        "`colour` mode only; default true. A region that \
                         `get_reference_analysis` measures as a gradient, or \
                         as a flat colour drawn translucent, is traced once \
                         as its outline and painted with that fill (paths \
                         then carry `region_id` and `appearance`). Set false \
                         to get plain flat-colour layers for everything.",
                    ),
                ),
            ],
            &["src"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let src = PathBuf::from(required_str(self.name(), input, "src")?);

        let refuse = |reason: String| crate::Error::InvalidToolInput {
            tool: self.name().to_owned(),
            reason,
        };

        let mode = match input.get("mode").and_then(Value::as_str) {
            None => crate::vectorize::TraceMode::default(),
            Some("silhouette") => crate::vectorize::TraceMode::Silhouette,
            Some("colour") => crate::vectorize::TraceMode::Colour,
            Some(other) => {
                return Err(refuse(format!(
                    "`mode` must be `silhouette` or `colour`, got `{other}`"
                )));
            }
        };
        // Refused rather than clamped or dropped: the schema promises these
        // bounds, so a value outside them is a mistake the model should be
        // told about, not one it should have quietly corrected to a
        // different trace than the one it asked for.
        // A key that is present but of the wrong type is refused like one out
        // of range: `"128"` silently becoming the default would trace
        // something other than what was asked for.
        let present = |key: &str| input.get(key).filter(|value| !value.is_null());
        let threshold = match present("threshold") {
            None => None,
            Some(raw) => {
                let value = raw.as_u64().ok_or_else(|| {
                    refuse(format!(
                        "`threshold` must be an integer from 0 to 255, got {raw}"
                    ))
                })?;
                Some(u8::try_from(value).map_err(|_| {
                    refuse(format!(
                        "`threshold` must be between 0 and 255, got {value}"
                    ))
                })?)
            }
        };
        let invert = match present("invert") {
            None => false,
            Some(raw) => raw
                .as_bool()
                .ok_or_else(|| refuse(format!("`invert` must be true or false, got {raw}")))?,
        };
        let max_colors = match present("max_colors") {
            None => None,
            Some(raw) => match raw.as_u64() {
                None => {
                    return Err(refuse(format!(
                        "`max_colors` must be an integer of at least 1, got {raw}"
                    )));
                }
                Some(0) => {
                    return Err(refuse(
                        "`max_colors` must be at least 1; omit it to let the clustering decide"
                            .to_owned(),
                    ));
                }
                Some(value) => usize::try_from(value).ok(),
            },
        };

        // Refused when present and not a boolean, not read as the default:
        // `"false"` silently becoming `true` would give the trace the caller
        // asked not to get.
        let appearance = match input.get("appearance") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(value)) => *value,
            Some(other) => {
                return Err(refuse(format!(
                    "`appearance` must be true or false, got {other}"
                )));
            }
        };

        // Reported before anything is read, so an unattached name is a clear
        // refusal, not a race.
        let reference = attached_reference(self.name(), project, &src)?;

        let resolved = project.resolve(&reference.src);

        // Before the file is read, for the reason `get_reference_image` gives.
        reference_mime_type_or_refuse(self.name(), &resolved)?;

        let bytes =
            std::fs::read(&resolved).map_err(|error| crate::Error::io(resolved.clone(), error))?;

        let traced = crate::vectorize::trace(
            &resolved,
            &bytes,
            crate::vectorize::TraceOptions {
                mode,
                threshold,
                invert,
                max_colors,
                appearance,
            },
        )?;
        let preview = crate::vectorize::preview(&traced.svg);

        Ok(ToolOutput::json(json!({
            "src": reference.src.display().to_string(),
            "resolved": resolved.display().to_string(),
            "trace": serde_json::to_value(&traced).expect("Traced always serialises"),
        }))
        .with_image(ToolImage::png(
            format!("{} traced ({mode} mode) preview", reference.src.display()),
            preview,
        )))
    }
}

/// Compare an attached reference against a rendered variant.
///
/// See [`crate::compare`] and ADR 019: renders `variant` at the reference's
/// own pixel dimensions — never the other way around, so nothing here is a
/// guess about alignment — and reports every signal separately rather than
/// folding them into one score, per issue #5's own design brief.
struct CompareReference;

impl Tool for CompareReference {
    fn name(&self) -> &'static str {
        "compare_reference"
    }

    fn description(&self) -> &'static str {
        "Compare an attached raster reference against a variant you have \
         written, in one call. Renders `variant` on a transparent background \
         at the reference's own pixel size — nothing is resampled — and \
         reports foreground-mask overlap, bounding-box and centroid offsets \
         (render minus reference), pixel error, an SSIM-based perceptual \
         similarity and edge overlap, each separately rather than as one \
         score. `appearance` separately reports fill mismatches (kind, \
         gradient, opacity, stroke) per region; its `findings` say what to \
         change. Returns four images, in order: the \
         reference, the render, an overlay (only in the reference: red \
         `#ff2060`; only in the render: cyan `#20c8ff`; in both: white) and a \
         difference heatmap. Measurement, not judgement: it says what \
         differs and by how much, never that you are finished. Use it after \
         each `write_variant` or `write_svg` when reproducing a `source` \
         reference, fix the largest offset, and call it again; use \
         `render_svg` when there is nothing to compare against."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "src",
                    string(
                        "Which reference to compare against. One of the \
                         paths from `get_references`, matched exactly.",
                    ),
                ),
                (
                    "variant",
                    string(
                        "Which variant to render and compare. One of the \
                         names from `get_variants`.",
                    ),
                ),
            ],
            &["src", "variant"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let src = PathBuf::from(required_str(self.name(), input, "src")?);
        let variant = required_str(self.name(), input, "variant")?.to_owned();

        // Same resolution as the other reference tools: reported before
        // anything is read, so an unattached name is a clear refusal, not a
        // race.
        let reference = attached_reference(self.name(), project, &src)?;

        let resolved = project.resolve(&reference.src);

        // Same extension-guessed mime type as `get_reference_image`, needed
        // to label the reference image correctly rather than always
        // claiming PNG — the render, overlay and difference images that
        // follow it genuinely are PNG.
        let mime_type = reference_mime_type_or_refuse(self.name(), &resolved)?;

        let reference_bytes =
            std::fs::read(&resolved).map_err(|error| crate::Error::io(resolved.clone(), error))?;

        // The canvas both images are compared on: the reference's own pixel
        // dimensions, decided once here rather than left for the model to
        // pick — see the `compare` module doc comment.
        let reference_probe = crate::analysis::classify(&resolved, &reference_bytes)?;

        let spec = RenderSpec {
            name: variant.clone(),
            variant: variant.clone(),
            width: reference_probe.width,
            height: reference_probe.height,
            format: Format::Png,
            // Always transparent, regardless of what the reference's own
            // background looks like: `compare` classifies the render's
            // foreground from its alpha channel, which only means something
            // if nothing else was painted behind it.
            background: Background::Transparent,
        };
        let asset = Renderer::new(project, RenderOptions::default())?.render(&spec)?;

        // A synthetic label, never read: the render was just produced from
        // bytes this call encoded itself, so a decode failure here would be
        // a bug in `compare`, not a fact about the project.
        let render_label = PathBuf::from(format!("<{variant} render>"));
        let (comparison, images) =
            crate::compare::compare(&resolved, &reference_bytes, &render_label, &asset.bytes)?;

        Ok(ToolOutput::json(
            serde_json::to_value(&comparison).expect("Comparison always serialises"),
        )
        .with_images([
            ToolImage::new(
                format!("{} ({})", reference.src.display(), reference.kind),
                mime_type,
                reference_bytes,
            ),
            ToolImage::png(
                format!("{variant} rendered at {}x{}", spec.width, spec.height),
                asset.bytes,
            ),
            ToolImage::png("overlay: reference vs. render silhouette", images.overlay),
            ToolImage::png("difference heatmap", images.difference),
        ]))
    }
}

/// Name the reconstruction workflow's phases, and, for reference/hybrid
/// work, recommend a construction strategy grounded in a reference's own
/// measurements.
///
/// See [`crate::workflow`] and ADR 021: this is guidance, not enforcement —
/// nothing here tracks whether a phase actually happened, and the agent
/// still decides how the SVG gets constructed.
struct GetWorkflow;

impl Tool for GetWorkflow {
    fn name(&self) -> &'static str {
        "get_workflow"
    }

    fn description(&self) -> &'static str {
        "Name the ordered phases of the reconstruction workflow for the kind \
         of work this is, and recommend a construction strategy for \
         reference/hybrid work. `from_scratch` (no reference exists): \
         construct, render, inspect, refine, validate — tracing is never \
         forced, and validating means checking the render itself. \
         `reference` (reproducing an attached source) and `hybrid` (mixing a \
         trace with hand-constructed geometry) share: inspect, analyse, \
         choose_strategy, construct, render, compare, refine, validate — \
         validate is only reachable after compare, since there is nothing to \
         validate a reproduction against without one. The tools for the \
         phases are `get_reference_image` (inspect), \
         `get_reference_analysis` (analyse), `get_reference_trace`, \
         `write_variant` and `write_svg` (construct), `render_svg` (render) \
         and `compare_reference` (compare). Call it before writing anything \
         for a `source` reference, with `src` so `strategy.recommended` \
         (`trace`, `construct` or `hybrid`) comes from that image's own \
         measurements; `hybrid` work always recommends `hybrid`. This is \
         guidance, not a checklist Shaipe enforces, and it does not track \
         which phases actually happened."
    }

    fn input_schema(&self) -> Value {
        object(
            &[
                (
                    "kind",
                    json!({
                        "type": "string",
                        "enum": ["from_scratch", "reference", "hybrid"],
                        "description": "Which kind of reconstruction work \
                         this is. `from_scratch`: no reference exists, built \
                         from a brief alone. `reference`: a `source` \
                         reference exists and the point is to reproduce it. \
                         `hybrid`: a reference exists, but the result is \
                         expected to mix a deterministic trace with \
                         hand-constructed geometry rather than reproduce it \
                         exactly.",
                    }),
                ),
                (
                    "src",
                    string(
                        "Only meaningful for `reference`/`hybrid`. One of \
                         the paths from `get_references`, matched exactly. \
                         When given, the response includes a \
                         `choose_strategy` recommendation grounded in that \
                         reference's own measurements.",
                    ),
                ),
            ],
            &["kind"],
        )
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        let refuse = |reason: String| crate::Error::InvalidToolInput {
            tool: self.name().to_owned(),
            reason,
        };

        let kind = match required_str(self.name(), input, "kind")? {
            "from_scratch" => crate::workflow::WorkflowKind::FromScratch,
            "reference" => crate::workflow::WorkflowKind::Reference,
            "hybrid" => crate::workflow::WorkflowKind::Hybrid,
            other => {
                return Err(refuse(format!(
                    "`kind` must be one of `from_scratch`, `reference` or \
                     `hybrid`, not `{other}`"
                )));
            }
        };

        let src = input.get("src").and_then(Value::as_str);

        if let (crate::workflow::WorkflowKind::FromScratch, Some(src)) = (kind, src) {
            return Err(refuse(format!(
                "`src` (`{src}`) does not apply to `from_scratch` work: \
                 there is no `choose_strategy` phase to recommend a \
                 strategy for, since no reference is being reproduced."
            )));
        }

        let workflow = crate::workflow::Workflow::for_kind(kind);
        let mut output = json!({
            "kind": kind.to_string(),
            "phases": workflow.phases.iter().map(ToString::to_string).collect::<Vec<_>>(),
        });

        if let Some(src) = src {
            let src = PathBuf::from(src);

            // Same resolution as the other reference tools: reported before
            // anything is read, so an unattached name is a clear refusal,
            // not a race.
            let reference = attached_reference(self.name(), project, &src)?;

            let resolved = project.resolve(&reference.src);
            reference_mime_type_or_refuse(self.name(), &resolved)?;
            let bytes = std::fs::read(&resolved)
                .map_err(|error| crate::Error::io(resolved.clone(), error))?;
            let analysis = crate::analysis::analyze(&resolved, &bytes)?;
            let recommendation = crate::workflow::recommend_strategy(kind, &analysis);

            output["strategy"] = serde_json::to_value(&recommendation)
                .expect("StrategyRecommendation always serialises");
        }

        Ok(ToolOutput::json(output))
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
         produce, not an approximation of it. The SVG source cannot tell you \
         what the artwork looks like, so look after every write. When \
         reproducing a `source` reference, `compare_reference` renders at the \
         reference's size and measures the difference, which this does not; \
         for small sizes use `render_grid`."
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
                    integer(
                        "Canvas height in pixels. Defaults to the width, giving a \
                         square; set both for any other shape.",
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
        // Echoed, as `render_grid` does: a transparent PNG shows as black or
        // white depending on the viewer, and the model should know which
        // background it asked for when a mark looks wrong.
        "background": spec.background.to_string(),
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
        Some(value) => value
            .parse()
            .map_err(|error: crate::Error| bad_colour(tool, "background", &error)),
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
         `<metadata>` block that makes it a Shaipe project. Read it before \
         any write: an edit not based on the current source silently discards \
         whatever else the file contains. To change one variant, find its \
         element by the `id` `get_variants` lists and send only that element \
         to `write_variant`; send the whole document back through \
         `write_svg` only for wider changes."
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
        "Render one variant at several square sizes at once and look at all \
         of them. Use it to check that a mark still reads when it is small: a \
         logo that works at 512 pixels often becomes an unreadable smudge at \
         16, and you learn that only by looking at it at 16. Not for \
         matching a reference, which `compare_reference` does at the \
         reference's own size."
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
                        "The square sizes to render, in pixels. Omit it, or send \
                         an empty list, for 512, 128, 64, 32 and 16.",
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
        "Replace the project's whole SVG source: to add a variant, or to change \
         several elements at once. To change one variant, `write_variant` is \
         smaller and safer. Send the whole document, not a fragment: call \
         `get_svg` first and return the complete file with your edit \
         applied. It is checked before anything is replaced, so a document \
         that does not parse, or that has lost its `<shaipe:project>` \
         metadata, is rejected and the project left exactly as it was. That \
         check parses; it does not render, so a variant whose element has \
         vanished is only found by the next `render_svg`. Accepted means \
         valid, not right: look at the result with `render_svg`, and for a \
         `source` reference run `compare_reference`. It changes the project \
         in memory, and reaches disk only if the server runs with `--write` \
         or the user saves."
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

        // A renderer must be constructible: this parses the XML and builds
        // the font database. It does *not* resolve or rasterise a variant, so
        // a document whose variant element has vanished is accepted here and
        // only fails at the next render — which is why the description says
        // so and the result points at `render_svg`. Rendering every variant
        // would be seconds to re-establish what an edit rarely breaks.
        Renderer::new(&candidate, RenderOptions::default()).map_err(invalid)?;

        *project = candidate;

        Ok(ToolOutput::json(json!({
            "bytes": source.len(),
            // Said out loud, every time. An agent that assumes it has saved
            // and has not will report work it did not do.
            "saved": false,
            "note": IN_MEMORY,
            "next": AFTER_A_WRITE,
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
         whole project. `variant` must already be declared (`get_variants`); \
         use `write_svg` to add one, or to change more than one element at \
         once. Send the complete replacement for that element — its opening \
         tag, attributes and closing tag, with the same `id` it already has — \
         found in `get_svg`. It is checked by isolating the variant before \
         anything is replaced, so a fragment that does not parse or that drops \
         the element's `id` is rejected and the project left exactly as it \
         was. Nothing is rasterised, so a fragment that isolates but draws \
         nothing, or draws the wrong thing, is accepted. Accepted means \
         well-formed, not right: look at the result with `render_svg`, and for a \
         `source` reference run `compare_reference`. It changes the project \
         in memory, and reaches disk only if the server runs with `--write` \
         or the user saves."
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

        // Spliced into the document as it now stands, not as it was read:
        // `source()` keeps the old metadata until saving, so splicing into it
        // and adopting the result would silently revert an unsaved prompt or
        // palette edit — the same data loss `get_svg` documents.
        let current = project.to_svg()?;
        let updated = document::replace_element(&current, name, &element, svg, project.path())
            .map_err(invalid)?;

        let candidate = Project::from_source(project.path(), updated).map_err(invalid)?;

        // The touched variant must still isolate — this catches a dropped or
        // renamed `id` and a fragment that does not parse. It does not run
        // `usvg`: `Format::Svg` returns the isolated document unrasterised.
        // Probed, not assumed: nine deliberately hostile fragments (a zero
        // `viewBox`, a self-referencing `<use>`, garbage CSS, a singular
        // transform…) all pass `usvg` too, so rasterising here would cost a
        // render and catch nothing this does not.
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
            "note": IN_MEMORY,
            "next": AFTER_A_WRITE,
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
         value; `restyled` in the result says how many attributes that was, \
         so 0 means the palette changed and no artwork followed. Bind an \
         element yourself with `write_variant` or `write_svg` by adding that \
         attribute, naming a colour already in the palette, next to its \
         `fill`/`stroke`; nothing restyles until an element names a colour \
         this way. Look at the result with `render_svg`."
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
            .map_err(|error| bad_colour(self.name(), "value", &error))?;

        // Restyled before the palette records the new value, so a failure
        // here — unreachable in practice, see `Project::restyle` — leaves the
        // document and the palette in their old, matching state rather than
        // updating one and not the other.
        //
        // Counted first, from the same binding rule the rewrite uses, so the
        // result can say how much artwork followed. Zero is otherwise
        // indistinguishable from success.
        let mut restyled = 0;
        if let Some(value) = value {
            restyled = project.bindings_of(name)?;
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
            "restyled": restyled,
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
         `inspiration` when `kind` is not given; use `source` for the thing \
         to reproduce. Only the fields given are changed on an existing \
         reference — give an empty `note` to clear it. The file does not \
         need to exist yet; the response's `present` says whether it \
         currently does, and the reference tools need it to. Once attached, \
         look at it with `get_reference_image`."
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
         know one of these, in particular the time, leave it out rather than \
         guessing."
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

    /// Two solid-colour discs, side by side on an opaque white background —
    /// same shape [`crate::vectorize`]'s own colour-mode tests build, reused
    /// here so this module does not need its own fixture file.
    fn two_coloured_discs_png(size: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba([255, 255, 255, 255]));
        let colours = [[220u8, 30, 30], [30, 90, 220]];
        let cell = size / colours.len() as u32;
        let radius = f64::from(cell) / 2.0 - 4.0;
        for (index, &[r, g, b]) in colours.iter().enumerate() {
            let cx = f64::from(index as u32 * cell) + f64::from(cell) / 2.0;
            let cy = f64::from(size) / 2.0;
            for y in 0..size {
                for x in 0..size {
                    let dx = f64::from(x) - cx;
                    let dy = f64::from(y) - cy;
                    if (dx * dx + dy * dy).sqrt() <= radius {
                        img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                    }
                }
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
        assert_eq!(output.value["trace"]["mode"], "silhouette");
        assert_eq!(output.value["trace"]["path_count"], 1);
        assert_eq!(output.value["trace"]["paths"].as_array().unwrap().len(), 1);
        assert!(
            output.value["trace"]["bounding_box"]["width"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        let svg = output.value["trace"]["svg"].as_str().unwrap();
        assert!(svg.contains("<path"), "{svg}");
        // A rendered preview travels alongside the JSON, so an agent can
        // inspect the result without first hand-grafting it into the
        // project — see ADR 020.
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].mime_type, "image/png");
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

        assert!(
            output.value["trace"]["svg"]
                .as_str()
                .unwrap()
                .contains("<path")
        );
    }

    #[test]
    fn get_reference_trace_refuses_an_option_of_the_wrong_type() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), ring_png(64)).unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        for bad in [
            json!({ "src": "mockup.png", "threshold": "128" }),
            json!({ "src": "mockup.png", "threshold": -1 }),
            json!({ "src": "mockup.png", "invert": "yes" }),
            json!({ "src": "mockup.png", "max_colors": 2.5 }),
        ] {
            let error = Registry::new()
                .call("get_reference_trace", &mut project, &bad)
                .unwrap_err();
            assert!(
                matches!(error, Error::InvalidToolInput { .. }),
                "{bad} should be refused, got {error:?}"
            );
        }
    }

    #[test]
    fn get_reference_trace_colour_mode_returns_multiple_paths_with_distinct_fills() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(
            directory.path().join("mockup.png"),
            two_coloured_discs_png(96),
        )
        .unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let output = Registry::new()
            .call(
                "get_reference_trace",
                &mut project,
                &json!({ "src": "mockup.png", "mode": "colour" }),
            )
            .unwrap();

        assert_eq!(output.value["trace"]["mode"], "colour");
        let path_count = output.value["trace"]["path_count"].as_u64().unwrap();
        assert!(path_count >= 2, "{}", output.value["trace"]);
        assert_eq!(output.images.len(), 1);
    }

    /// A red-to-blue ramp across a square, on a transparent canvas.
    fn gradient_png() -> Vec<u8> {
        let mut img = image::RgbaImage::new(64, 64);
        for y in 12..52 {
            for x in 12..52 {
                let t = f64::from(x - 12) / 39.0;
                img.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        (200.0 - 170.0 * t) as u8,
                        (30.0 + 10.0 * t) as u8,
                        (30.0 + 180.0 * t) as u8,
                        255,
                    ]),
                );
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

    fn trace_gradient(arguments: Value) -> Result<ToolOutput> {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), gradient_png()).unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        Registry::new().call("get_reference_trace", &mut project, &arguments)
    }

    #[test]
    fn get_reference_trace_keeps_a_gradient_as_one_path_that_names_its_region() {
        let output = trace_gradient(json!({ "src": "mockup.png", "mode": "colour" })).unwrap();

        let trace = &output.value["trace"];
        assert_eq!(trace["path_count"], 1, "{trace}");
        let path = &trace["paths"][0];
        assert_eq!(path["region_id"], 0, "{path}");
        assert_eq!(
            path["appearance"]["fill"]["kind"], "linear_gradient",
            "{path}"
        );
        assert!(trace["svg"].as_str().unwrap().contains("linearGradient"));
    }

    #[test]
    fn get_reference_trace_with_appearance_off_returns_the_flat_layers() {
        let output =
            trace_gradient(json!({ "src": "mockup.png", "mode": "colour", "appearance": false }))
                .unwrap();

        let trace = &output.value["trace"];
        assert!(trace["path_count"].as_u64().unwrap() > 3, "{trace}");
        assert!(trace["paths"][0].get("region_id").is_none(), "{trace}");
    }

    #[test]
    fn get_reference_trace_refuses_an_appearance_that_is_not_a_boolean() {
        let error =
            trace_gradient(json!({ "src": "mockup.png", "mode": "colour", "appearance": "false" }))
                .unwrap_err();

        assert!(matches!(error, Error::InvalidToolInput { .. }), "{error}");
        assert!(error.to_string().contains("appearance"), "{error}");
    }

    #[test]
    fn get_reference_trace_of_an_invalid_mode_is_refused() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_trace",
                &mut project,
                &json!({ "src": "mockup.png", "mode": "watercolour" }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::InvalidToolInput { .. }));
        assert!(error.to_string().contains("mode"), "{error}");
    }

    #[test]
    fn get_reference_analysis_reports_measurements_for_an_attached_image() {
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
                "get_reference_analysis",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap();

        assert_eq!(output.value["src"], "mockup.png");
        let analysis = &output.value["analysis"];
        assert_eq!(analysis["dimensions"]["width"], 64);
        assert_eq!(analysis["dimensions"]["height"], 64);
        assert_eq!(analysis["region_count"], 1);
        assert_eq!(analysis["hole_count"], 1);
        assert_eq!(
            analysis["holes"][0]["enclosed_by"],
            analysis["regions"][0]["id"]
        );

        // No mutation: this is a `get_` tool.
        assert!(
            !Registry::new()
                .get("get_reference_analysis")
                .unwrap()
                .mutates()
        );
    }

    #[test]
    fn get_reference_analysis_tells_a_gradient_from_a_flat_fill() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();

        // A square with a margin: one ramps left to right, the other is flat.
        let square = |ramp: bool| {
            let mut img = image::RgbaImage::new(64, 64);
            for y in 12..52u32 {
                for x in 12..52u32 {
                    let shade = if ramp { (x - 12) * 6 } else { 90 };
                    img.put_pixel(x, y, image::Rgba([shade as u8, 40, 120, 255]));
                }
            }
            let mut bytes = Vec::new();
            img.write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
            bytes
        };
        std::fs::write(directory.path().join("ramp.png"), square(true)).unwrap();
        std::fs::write(directory.path().join("flat.png"), square(false)).unwrap();

        let mut project = Project::open(&path).unwrap();
        for name in ["ramp.png", "flat.png"] {
            project
                .metadata_mut()
                .references
                .push(Reference::new(name, ReferenceKind::Source));
        }

        let kind_of = |project: &mut Project, src: &str| {
            let output = Registry::new()
                .call("get_reference_analysis", project, &json!({ "src": src }))
                .unwrap();
            output.value["analysis"]["appearance"]["regions"][0]["fill"]["kind"].clone()
        };
        assert_eq!(kind_of(&mut project, "ramp.png"), "linear_gradient");
        assert_eq!(kind_of(&mut project, "flat.png"), "flat");
    }

    #[test]
    fn get_reference_analysis_of_an_unknown_src_lists_the_ones_that_are_attached() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_reference_analysis",
                &mut project,
                &json!({ "src": "watermark.png" }),
            )
            .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("mockup.png"), "{rendered}");
        assert!(matches!(error, Error::InvalidToolInput { .. }));
    }

    #[test]
    fn get_reference_analysis_of_bytes_that_are_not_an_image_reports_why() {
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
                "get_reference_analysis",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::AnalysisDecode { .. }));
    }

    #[test]
    fn compare_reference_returns_four_images_and_the_comparison_report() {
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
                "compare_reference",
                &mut project,
                &json!({ "src": "mockup.png", "variant": "icon" }),
            )
            .unwrap();

        assert_eq!(output.value["canvas"]["width"], 64);
        assert_eq!(output.value["canvas"]["height"], 64);
        assert!(output.value["foreground"]["intersection_over_union"].is_number());
        assert!(output.value["perceptual_similarity"]["score"].is_number());

        // Reference, render, overlay, difference — in that order, per the
        // issue's own "Output" list.
        let [reference, render, overlay, difference] = &output.images[..] else {
            panic!("expected exactly four images, got {}", output.images.len());
        };
        assert_eq!(reference.mime_type, "image/png");
        assert_eq!(reference.label, "mockup.png (source)");
        assert_eq!(render.mime_type, "image/png");
        assert_eq!(render.label, "icon rendered at 64x64");
        assert_eq!(overlay.mime_type, "image/png");
        assert_eq!(difference.mime_type, "image/png");

        // No mutation: this is a `get_`-shaped, read-only tool.
        assert!(!Registry::new().get("compare_reference").unwrap().mutates());
    }

    #[test]
    fn compare_reference_of_an_unknown_src_lists_the_ones_that_are_attached() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "compare_reference",
                &mut project,
                &json!({ "src": "watermark.png", "variant": "icon" }),
            )
            .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("mockup.png"), "{rendered}");
        assert!(matches!(error, Error::InvalidToolInput { .. }));
    }

    #[test]
    fn compare_reference_of_an_unknown_variant_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), ring_png(64)).unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "compare_reference",
                &mut project,
                &json!({ "src": "mockup.png", "variant": "watermark" }),
            )
            .unwrap_err();

        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("icon"), "{rendered}");
        assert!(rendered.contains("wordmark"), "{rendered}");
    }

    #[test]
    fn get_workflow_lists_from_scratch_phases_without_a_source() {
        let value = value("get_workflow", json!({ "kind": "from_scratch" }));
        assert_eq!(value["kind"], "from_scratch");
        assert_eq!(
            value["phases"],
            json!(["construct", "render", "inspect", "refine", "validate"])
        );
        assert_eq!(value.get("strategy"), None);
    }

    #[test]
    fn get_workflow_lists_reference_phases_including_compare_before_validate() {
        let value = value("get_workflow", json!({ "kind": "reference" }));
        assert_eq!(value["kind"], "reference");
        let phases: Vec<&str> = value["phases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|phase| phase.as_str().unwrap())
            .collect();
        let compare = phases.iter().position(|phase| *phase == "compare").unwrap();
        let validate = phases
            .iter()
            .position(|phase| *phase == "validate")
            .unwrap();
        assert!(compare < validate, "{phases:?}");
        // No mutation: this is a `get_`-shaped, read-only tool.
        assert!(!Registry::new().get("get_workflow").unwrap().mutates());
    }

    #[test]
    fn get_workflow_recommends_tracing_a_measurable_source() {
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
                "get_workflow",
                &mut project,
                &json!({ "kind": "reference", "src": "mockup.png" }),
            )
            .unwrap();

        assert_eq!(output.value["strategy"]["recommended"], "trace");
        assert_eq!(output.value["strategy"]["region_count"], 1);
    }

    #[test]
    fn get_workflow_recommends_mixing_for_hybrid_work_regardless_of_measurements() {
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
                "get_workflow",
                &mut project,
                &json!({ "kind": "hybrid", "src": "mockup.png" }),
            )
            .unwrap();

        assert_eq!(output.value["strategy"]["recommended"], "hybrid");
    }

    #[test]
    fn get_workflow_refuses_a_source_for_from_scratch_work() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_workflow",
                &mut project,
                &json!({ "kind": "from_scratch", "src": "mockup.png" }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::InvalidToolInput { .. }));
    }

    #[test]
    fn get_workflow_refuses_an_unknown_kind() {
        let error = Registry::new()
            .call(
                "get_workflow",
                &mut fixtures::project(),
                &json!({ "kind": "improvised" }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::InvalidToolInput { .. }));
    }

    #[test]
    fn get_workflow_refuses_an_unattached_reference() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        let error = Registry::new()
            .call(
                "get_workflow",
                &mut project,
                &json!({ "kind": "reference", "src": "watermark.png" }),
            )
            .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("mockup.png"), "{rendered}");
        assert!(matches!(error, Error::InvalidToolInput { .. }));
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
    fn write_variant_preserves_metadata_edited_since_the_project_was_read() {
        // `source()` still carries the metadata as it was read, so splicing
        // into it and adopting the result reverted every unsaved metadata
        // edit: the same data loss `get_svg` documents for `write_svg`.
        let mut project = fixtures::project();
        project.metadata_mut().prompt = Some("a wordless circular mark".to_owned());

        let registry = Registry::new();
        registry
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "accent", "value": "#0066ff" }),
            )
            .unwrap();

        registry
            .call(
                "write_variant",
                &mut project,
                &json!({
                    "variant": "icon",
                    "svg": r##"<symbol id="icon" viewBox="0 0 64 64"><circle r="32" cx="32" cy="32" fill="#f05032"/></symbol>"##,
                }),
            )
            .unwrap();

        assert_eq!(
            project.metadata().prompt.as_deref(),
            Some("a wordless circular mark"),
            "writing a variant reverted the prompt"
        );
        assert_eq!(
            project.metadata().palette.colours()[0].value.to_string(),
            "#0066ff",
            "writing a variant reverted a palette edit"
        );
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
    fn set_palette_colour_says_how_many_bindings_it_restyled() {
        // Zero is what a colour nothing is bound to reports, and without the
        // count it is indistinguishable from the palette and the drawing
        // having both followed.
        let mut project = Project::from_source("logo.svg", BOUND_PROJECT.to_owned()).unwrap();

        let output = Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "accent", "value": "#0066ff" }),
            )
            .unwrap();
        assert_eq!(output.value["restyled"], 1);

        let output = Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "unbound", "value": "#123456" }),
            )
            .unwrap();
        assert_eq!(output.value["restyled"], 0);

        let output = Registry::new()
            .call(
                "set_palette_colour",
                &mut project,
                &json!({ "name": "accent", "role": "primary" }),
            )
            .unwrap();
        assert_eq!(output.value["restyled"], 0);
    }

    #[test]
    fn a_colour_that_is_refused_says_what_a_colour_looks_like() {
        // The parser's help names the accepted formats. Re-wrapping the error
        // by its message alone used to drop it.
        for (tool, input, argument) in [
            (
                "render_svg",
                json!({ "variant": "icon", "background": "octarine" }),
                "background",
            ),
            (
                "set_palette_colour",
                json!({ "name": "accent", "value": "octarine" }),
                "value",
            ),
        ] {
            let rendered = call(tool, input).unwrap_err().to_string();
            assert!(rendered.contains(argument), "{rendered}");
            assert!(rendered.contains("#rrggbb"), "{tool}: {rendered}");
        }
    }

    #[test]
    fn a_write_points_at_the_render_that_will_show_whether_it_worked() {
        // The step after a write is a look, and the result is what the model
        // has in front of it when it decides whether to take that step.
        let mut project = fixtures::project();
        let registry = Registry::new();
        let names = registry.names();

        for (tool, input) in [
            ("write_svg", json!({ "source": fixtures::PROJECT })),
            (
                "write_variant",
                json!({
                    "variant": "icon",
                    "svg": r##"<symbol id="icon" viewBox="0 0 64 64"><circle r="32" cx="32" cy="32" fill="#f05032"/></symbol>"##,
                }),
            ),
        ] {
            let output = registry.call(tool, &mut project, &input).unwrap();
            let next = output.value["next"]
                .as_str()
                .expect("a write says what is next");

            assert!(next.contains("`render_svg`"), "{tool}: {next}");
            assert!(next.contains("`compare_reference`"), "{tool}: {next}");
            // `source` is a kind of reference, not a tool; every tool name has
            // an underscore.
            for name in next
                .split('`')
                .skip(1)
                .step_by(2)
                .filter(|n| n.contains('_'))
            {
                assert!(names.iter().any(|n| n == name), "{tool} names `{name}`");
            }
        }
    }

    #[test]
    fn a_write_does_not_claim_the_file_was_not_written_when_the_server_may_save() {
        // Under `shaipe mcp --write` the session saves after a mutating call,
        // so "nothing has been written" would be false there. The note says
        // what this call did and when the file is touched.
        let output = Registry::new()
            .call(
                "write_svg",
                &mut fixtures::project(),
                &json!({ "source": fixtures::PROJECT }),
            )
            .unwrap();

        let note = output.value["note"].as_str().unwrap();
        assert!(note.contains("--write"), "{note}");
        assert!(!note.contains("Nothing has been written"), "{note}");
    }

    #[test]
    fn write_variant_asks_for_the_element_and_not_the_document() {
        // The one help text used to tell every writer to resend the whole
        // document, which is the opposite of what `write_variant` wants.
        let mut project = fixtures::project();
        let error = Registry::new()
            .call(
                "write_variant",
                &mut project,
                &json!({ "variant": "icon", "svg": "<symbol id=\"icon\"" }),
            )
            .unwrap_err();
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("only the"), "{rendered}");
        assert!(rendered.contains("element"), "{rendered}");
        assert!(!rendered.contains("all of it back"), "{rendered}");

        let error = Registry::new()
            .call(
                "write_svg",
                &mut project,
                &json!({ "source": "<svg><circle r=\"5\"" }),
            )
            .unwrap_err();
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("all of it back"), "{rendered}");
    }

    #[test]
    fn render_svg_says_which_background_it_drew_on() {
        let value = value(
            "render_svg",
            json!({ "variant": "icon", "width": 16, "background": "#ffffff" }),
        );
        assert_eq!(value["background"], "#ffffff");

        let value = self::value("render_svg", json!({ "variant": "icon", "width": 16 }));
        assert_eq!(value["background"], "transparent");
    }

    #[test]
    fn get_reference_image_reports_the_pixel_dimensions_of_a_real_image() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), ring_png(48)).unwrap();
        std::fs::write(directory.path().join("broken.png"), b"not a png").unwrap();

        let mut project = Project::open(&path).unwrap();
        for src in ["mockup.png", "broken.png"] {
            project
                .metadata_mut()
                .references
                .push(Reference::new(src, ReferenceKind::Source));
        }

        let output = Registry::new()
            .call(
                "get_reference_image",
                &mut project,
                &json!({ "src": "mockup.png" }),
            )
            .unwrap();
        assert_eq!(output.value["dimensions"]["width"], 48);
        assert_eq!(output.value["dimensions"]["height"], 48);

        // Bytes that do not decode are still handed over, as they always
        // were; the dimensions are simply absent.
        let output = Registry::new()
            .call(
                "get_reference_image",
                &mut project,
                &json!({ "src": "broken.png" }),
            )
            .unwrap();
        assert_eq!(output.value["dimensions"], Value::Null);
    }

    #[test]
    fn a_reference_that_cannot_be_shown_is_refused_for_that_before_it_is_read() {
        // A PDF that is not on disk is refused for what it is, not for
        // whatever reading it said first.
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("brief.pdf", ReferenceKind::Source));

        // All five tools that read a reference's pixels, not only the two that
        // hand it to a model as an image: ADR 023 promises the refusal for
        // every one of them.
        for tool in [
            "get_reference_image",
            "get_reference_analysis",
            "get_reference_trace",
            "compare_reference",
            "get_workflow",
        ] {
            let error = Registry::new()
                .call(
                    tool,
                    &mut project,
                    &json!({ "src": "brief.pdf", "variant": "icon", "kind": "reference" }),
                )
                .unwrap_err();

            assert!(
                matches!(error, Error::InvalidToolInput { .. }),
                "{tool}: {error:?}"
            );
            assert!(error.to_string().contains("set_reference"), "{error}");
        }
    }

    #[test]
    fn get_reference_trace_refuses_bounds_its_schema_promises() {
        // Dropped silently, a threshold of 300 traced at the default and told
        // the model nothing about it.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();
        std::fs::write(directory.path().join("mockup.png"), ring_png(64)).unwrap();

        let mut project = Project::open(&path).unwrap();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        for (input, argument) in [
            (
                json!({ "src": "mockup.png", "threshold": 300 }),
                "threshold",
            ),
            (
                json!({ "src": "mockup.png", "max_colors": 0 }),
                "max_colors",
            ),
        ] {
            let error = Registry::new()
                .call("get_reference_trace", &mut project, &input)
                .unwrap_err();

            assert!(matches!(error, Error::InvalidToolInput { .. }), "{error:?}");
            assert!(error.to_string().contains(argument), "{error}");
        }
    }

    #[test]
    fn an_unattached_reference_says_where_the_attached_ones_and_the_way_to_add_one_are() {
        let mut project = fixtures::project();
        project
            .metadata_mut()
            .references
            .push(Reference::new("mockup.png", ReferenceKind::Source));

        for tool in [
            "get_reference_image",
            "get_reference_analysis",
            "get_reference_trace",
            "compare_reference",
        ] {
            let error = Registry::new()
                .call(
                    tool,
                    &mut project,
                    &json!({ "src": "watermark.png", "variant": "icon" }),
                )
                .unwrap_err();

            let rendered = error.to_string();
            assert!(rendered.contains("mockup.png"), "{tool}: {rendered}");
            assert!(rendered.contains("set_reference"), "{tool}: {rendered}");
            assert!(rendered.contains("get_references"), "{tool}: {rendered}");
        }
    }

    #[test]
    fn a_trace_of_nothing_offers_advice_that_holds_in_either_mode() {
        let rendered = format!(
            "{:?}",
            miette::Report::new(Error::EmptyTrace {
                path: PathBuf::from("mockup.png"),
            })
        );
        assert!(rendered.contains("silhouette"), "{rendered}");
        assert!(rendered.contains("colour"), "{rendered}");
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
