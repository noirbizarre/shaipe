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
use crate::preview::Image;
use crate::project::{Format, Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};
use crate::tools::{Parameter, Tool, required_str};

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

    fn parameters(&self) -> &'static [Parameter] {
        &[]
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<Value> {
        Ok(serde_json::to_value(Report::of(project)).expect("a report is always serialisable"))
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

    fn parameters(&self) -> &'static [Parameter] {
        &[]
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<Value> {
        let metadata = project.metadata();
        Ok(Value::Array(
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
        ))
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

    fn parameters(&self) -> &'static [Parameter] {
        &[]
    }

    fn call(&self, project: &mut Project, _input: &Value) -> Result<Value> {
        Ok(Value::Array(
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
        ))
    }
}

/// Rasterise a variant.
struct Render;

impl Tool for Render {
    fn name(&self) -> &'static str {
        "render"
    }

    fn description(&self) -> &'static str {
        "Render a variant of the project to a PNG and return it as base64. \
         This is how to see what the SVG actually looks like: rendering is \
         local and exact, so the image returned is the asset a build would \
         produce, not an approximation of it."
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[
            Parameter {
                name: "variant",
                description: "Which variant to draw. One of the names from `list_variants`.",
                required: true,
            },
            Parameter {
                name: "width",
                description: "Canvas width in pixels. Defaults to 512.",
                required: false,
            },
            Parameter {
                name: "height",
                description: "Canvas height in pixels. Defaults to the width.",
                required: false,
            },
        ]
    }

    fn call(&self, project: &mut Project, input: &Value) -> Result<Value> {
        let variant = required_str(self.name(), input, "variant")?;

        // A default rather than a required argument: a model asking to see a
        // logo has no basis for choosing a size, and being forced to invent
        // one adds a decision that can be wrong.
        let dimension = |key: &str, fallback: u32| {
            input
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(fallback)
        };
        let width = dimension("width", 512);

        let spec = RenderSpec {
            name: variant.to_owned(),
            variant: variant.to_owned(),
            width,
            height: dimension("height", width),
            format: Format::Png,
            background: crate::project::Background::Transparent,
        };

        let asset = Renderer::new(project, RenderOptions::default())?.render(&spec)?;
        // Decoded and re-encoded through `Image` so that the dimensions
        // reported are the ones actually in the PNG rather than the ones that
        // were asked for.
        let image = Image::from_png(&asset.bytes)?;

        use base64::Engine as _;
        Ok(json!({
            "variant": variant,
            "width": image.width(),
            "height": image.height(),
            "mime_type": "image/png",
            "base64": base64::engine::general_purpose::STANDARD.encode(&asset.bytes),
        }))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;
    use crate::tools::Registry;

    fn call(name: &str, input: Value) -> Result<Value> {
        Registry::new().call(name, &mut fixtures::project(), &input)
    }

    #[test]
    fn inspect_project_returns_the_same_report_the_cli_prints() {
        // One description of a project, not two that can disagree.
        let value = call("inspect_project", Value::Null).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["variants"][1]["element"], "mark-wide");
    }

    #[test]
    fn list_variants_says_which_variant_the_document_root_draws() {
        let value = call("list_variants", Value::Null).unwrap();
        assert_eq!(value[0]["name"], "icon");
        assert_eq!(value[0]["primary"], true);
        assert_eq!(value[1]["primary"], false);
    }

    #[test]
    fn inspect_palette_returns_every_colour_with_its_role() {
        let value = call("inspect_palette", Value::Null).unwrap();
        assert_eq!(value[0]["value"], "#f05032");
        assert_eq!(value[0]["role"], "accent");
        assert_eq!(value[1]["role"], Value::Null);
    }

    #[test]
    fn render_returns_a_decodable_png_of_the_requested_size() {
        let value = call("render", json!({ "variant": "icon", "width": 64 })).unwrap();

        assert_eq!(value["width"], 64);
        assert_eq!(value["height"], 64);
        assert_eq!(value["mime_type"], "image/png");

        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(value["base64"].as_str().unwrap())
            .unwrap();
        assert!(tiny_skia::Pixmap::decode_png(&bytes).is_ok());
    }

    #[test]
    fn render_defaults_to_a_square_so_a_model_need_not_invent_a_size() {
        let value = call("render", json!({ "variant": "icon" })).unwrap();
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
}
