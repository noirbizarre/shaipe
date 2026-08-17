//! The tools Shaipe implements.
//!
//! All read-only, and all thin: each one is a name and a shape over an
//! operation the library already performs. That is the intended proportion —
//! a tool that contained logic of its own would be logic the CLI and the TUI
//! could not reach.
//!
//! The obvious gap is anything that *changes* the project. Those are the next
//! set, not a missing part of this one: writing a tool that mutates an SVG
//! before there is a way to review what it did would be building the wrong
//! half first.

use serde_json::{Value, json};

use crate::error::Result;
use crate::inspect::Report;
use crate::project::{Background, Format, Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};
use crate::tools::{
    Tool, ToolImage, ToolOutput, integer, integers, object, optional_u32, required_str, string,
};

/// Every tool Shaipe implements.
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(GetProject),
        Box::new(GetVariants),
        Box::new(GetPalette),
        Box::new(GetSvg),
        Box::new(RenderSvg),
        Box::new(RenderGrid),
        Box::new(WriteSvg),
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
        // Whole, never truncated. A model handed half a document writes back
        // half a document, and `write_svg` would then be right to reject it —
        // having spent a turn on a failure this tool caused.
        Ok(ToolOutput::json(json!({
            "path": project.path().display().to_string(),
            "bytes": project.source().len(),
            "source": project.source(),
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

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
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
    fn only_write_svg_declares_that_it_mutates() {
        // `get_` and `render_` promise no mutation in ADR 007, and
        // `Tool::mutates` says the same thing in code. This is what stops the
        // two from disagreeing.
        let mutating: Vec<_> = Registry::new()
            .tools()
            .filter(|tool| tool.mutates())
            .map(Tool::name)
            .collect();
        assert_eq!(mutating, ["write_svg"]);
    }

    #[test]
    fn get_svg_returns_the_document_byte_for_byte() {
        // Not summarised, not truncated, not re-serialised. A model that gets
        // anything other than the exact bytes cannot edit them and send them
        // back.
        let value = value("get_svg", Value::Null);
        assert_eq!(value["source"].as_str().unwrap(), fixtures::PROJECT);
        assert_eq!(value["bytes"], fixtures::PROJECT.len());
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
}
