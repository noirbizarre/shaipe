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
use crate::project::{Format, Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};
use crate::tools::{
    Tool, ToolImage, ToolOutput, integer, object, optional_u32, required_str, string,
};

/// Every tool Shaipe implements.
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(InspectProject),
        Box::new(ListVariants),
        Box::new(InspectPalette),
        Box::new(Render),
    ]
}

/// Describe the whole project.
struct InspectProject;

impl Tool for InspectProject {
    fn name(&self) -> &'static str {
        "inspect_project"
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
struct ListVariants;

impl Tool for ListVariants {
    fn name(&self) -> &'static str {
        "list_variants"
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
struct InspectPalette;

impl Tool for InspectPalette {
    fn name(&self) -> &'static str {
        "inspect_palette"
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
struct Render;

impl Tool for Render {
    fn name(&self) -> &'static str {
        "render"
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
                    string("Which variant to draw. One of the names from `list_variants`."),
                ),
                ("width", integer("Canvas width in pixels. Defaults to 512.")),
                (
                    "height",
                    integer("Canvas height in pixels. Defaults to the width, giving a square."),
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
            background: crate::project::Background::Transparent,
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
    fn inspect_project_returns_the_same_report_the_cli_prints() {
        // One description of a project, not two that can disagree.
        let value = value("inspect_project", Value::Null);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["variants"][1]["element"], "mark-wide");
    }

    #[test]
    fn list_variants_says_which_variant_the_document_root_draws() {
        let value = value("list_variants", Value::Null);
        assert_eq!(value[0]["name"], "icon");
        assert_eq!(value[0]["primary"], true);
        assert_eq!(value[1]["primary"], false);
    }

    #[test]
    fn inspect_palette_returns_every_colour_with_its_role() {
        let value = value("inspect_palette", Value::Null);
        assert_eq!(value[0]["value"], "#f05032");
        assert_eq!(value[0]["role"], "accent");
        assert_eq!(value[1]["role"], Value::Null);
    }

    #[test]
    fn render_returns_a_decodable_png_of_the_requested_size() {
        let output = call("render", json!({ "variant": "icon", "width": 64 })).unwrap();

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
    fn render_defaults_to_a_square_so_a_model_need_not_invent_a_size() {
        let value = value("render", json!({ "variant": "icon" }));
        assert_eq!(value["width"], 512);
        assert_eq!(value["height"], 512);
    }

    #[test]
    fn render_without_a_variant_says_which_argument_is_missing() {
        let error = call("render", json!({ "width": 64 })).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("variant"), "{rendered}");
    }

    #[test]
    fn render_of_an_undeclared_variant_lists_the_ones_that_exist() {
        // The recovery path for a model that guessed a name: the error tells
        // it what it should have called instead.
        let error = call("render", json!({ "variant": "watermark" })).unwrap_err();
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("icon"), "{rendered}");
        assert!(rendered.contains("wordmark"), "{rendered}");
    }

    #[test]
    fn no_tool_yet_changes_the_project() {
        // The whole set is read-only at this point. When that stops being
        // true, this test is the one that says so out loud.
        for tool in Registry::new().tools() {
            assert!(!tool.mutates(), "`{}` claims to mutate", tool.name());
        }
    }
}
