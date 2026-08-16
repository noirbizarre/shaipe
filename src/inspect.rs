//! A serialisable description of a project.
//!
//! This is the answer to "what is in this file?", in a shape that is equally
//! useful to a person reading `shaipe inspect` and to an agent parsing
//! `shaipe inspect --format json`. Both go through here, so the two can never
//! drift apart and start describing different projects.
//!
//! It is a *view*, not the model: it flattens, it resolves relative paths, and
//! it is free to change shape in ways [`crate::project::Metadata`] is not.
//! Consumers that need stability should pin to the schema version the project
//! declares, which this reports.

use std::path::PathBuf;

use serde::Serialize;

use crate::project::Project;
use crate::project::metadata::SCHEMA_VERSION;

/// Everything `shaipe inspect` reports about a project.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The project file.
    pub path: PathBuf,
    /// The metadata schema revision this report was produced by.
    pub schema_version: u32,
    /// The variant the document's root draws.
    pub primary: Option<String>,
    /// What the project is meant to be.
    pub prompt: Option<String>,
    /// How the current state was produced, if anything recorded it.
    pub generation: Option<GenerationReport>,
    /// The project's colours.
    pub palette: Vec<ColourReport>,
    /// Fonts the document's text depends on.
    pub fonts: Vec<FontReport>,
    /// Renderable parts of the document.
    pub variants: Vec<VariantReport>,
    /// Images and documents the project was informed by.
    pub references: Vec<ReferenceReport>,
    /// Assets the project declares.
    pub renders: Vec<RenderReport>,
}

/// How a project's current state was produced.
#[derive(Debug, Clone, Serialize)]
pub struct GenerationReport {
    /// The agent that produced it.
    pub agent: Option<String>,
    /// The model it used.
    pub model: Option<String>,
    /// When, as the agent recorded it.
    pub at: Option<String>,
}

/// One colour in the palette.
#[derive(Debug, Clone, Serialize)]
pub struct ColourReport {
    /// How the project refers to it.
    pub name: String,
    /// Its value, as CSS hex.
    pub value: String,
    /// What it is for, when the project says.
    pub role: Option<String>,
}

/// One font the project supplies.
#[derive(Debug, Clone, Serialize)]
pub struct FontReport {
    /// The family it provides.
    pub family: String,
    /// Where the project says it is, verbatim.
    pub src: PathBuf,
    /// Where that resolves to, and whether anything is there. An agent asked
    /// to fix a font problem needs both.
    pub resolved: PathBuf,
    /// Whether the file exists.
    pub present: bool,
}

/// One renderable part of the document.
#[derive(Debug, Clone, Serialize)]
pub struct VariantReport {
    /// The name render specifications use.
    pub name: String,
    /// The element in the document that draws it.
    pub element: String,
}

/// One file attached to the project for context.
#[derive(Debug, Clone, Serialize)]
pub struct ReferenceReport {
    /// Where the project says it is, verbatim.
    pub src: PathBuf,
    /// Where that resolves to.
    pub resolved: PathBuf,
    /// Why it is attached.
    pub kind: String,
    /// What it is, in the author's words.
    pub note: Option<String>,
    /// Whether the file exists.
    pub present: bool,
}

/// One asset the project declares.
#[derive(Debug, Clone, Serialize)]
pub struct RenderReport {
    /// The output file's stem.
    pub name: String,
    /// The file it writes.
    pub file_name: String,
    /// Which variant it draws.
    pub variant: String,
    /// Canvas width, in pixels.
    pub width: u32,
    /// Canvas height, in pixels.
    pub height: u32,
    /// How it is encoded.
    pub format: String,
    /// What it is drawn on.
    pub background: String,
}

impl Report {
    /// Describe a project.
    #[must_use]
    pub fn of(project: &Project) -> Self {
        let metadata = project.metadata();

        Self {
            path: project.path().to_path_buf(),
            schema_version: SCHEMA_VERSION,
            primary: metadata.primary.clone(),
            prompt: metadata.prompt.clone(),
            generation: (!metadata.generation.is_empty()).then(|| GenerationReport {
                agent: metadata.generation.agent.clone(),
                model: metadata.generation.model.clone(),
                at: metadata.generation.at.clone(),
            }),
            palette: metadata
                .palette
                .colours()
                .iter()
                .map(|colour| ColourReport {
                    name: colour.name.clone(),
                    value: colour.value.to_string(),
                    role: colour.role.as_ref().map(ToString::to_string),
                })
                .collect(),
            fonts: metadata
                .fonts
                .iter()
                .map(|font| {
                    let resolved = project.resolve(&font.src);
                    FontReport {
                        family: font.family.clone(),
                        src: font.src.clone(),
                        present: resolved.is_file(),
                        resolved,
                    }
                })
                .collect(),
            variants: metadata
                .variants
                .iter()
                .map(|variant| VariantReport {
                    name: variant.name.clone(),
                    element: variant.element.clone(),
                })
                .collect(),
            references: metadata
                .references
                .iter()
                .map(|reference| {
                    let resolved = project.resolve(&reference.src);
                    ReferenceReport {
                        src: reference.src.clone(),
                        kind: reference.kind.to_string(),
                        note: reference.note.clone(),
                        present: resolved.exists(),
                        resolved,
                    }
                })
                .collect(),
            renders: metadata
                .renders
                .iter()
                .map(|spec| RenderReport {
                    name: spec.name.clone(),
                    file_name: spec.file_name(),
                    variant: spec.variant.clone(),
                    width: spec.width,
                    height: spec.height,
                    format: spec.format.to_string(),
                    background: spec.background.to_string(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;

    #[test]
    fn a_report_describes_every_declared_part_of_a_project() {
        let report = Report::of(&fixtures::project());

        assert_eq!(report.primary.as_deref(), Some("icon"));
        assert_eq!(report.palette.len(), 2);
        assert_eq!(report.palette[0].value, "#f05032");
        assert_eq!(report.palette[0].role.as_deref(), Some("accent"));
        assert_eq!(report.variants[1].element, "mark-wide");
        assert_eq!(report.renders[0].file_name, "favicon-32.png");
    }

    #[test]
    fn generation_is_absent_rather_than_empty_when_nothing_has_recorded_it() {
        // An agent must be able to tell "never generated" from "generated by
        // something that declined to say what it was".
        assert!(Report::of(&fixtures::project()).generation.is_none());
    }

    #[test]
    fn a_report_serialises_to_json() {
        let report = Report::of(&fixtures::project());
        let json: serde_json::Value = serde_json::to_value(&report).unwrap();

        assert_eq!(json["schema_version"], SCHEMA_VERSION);
        assert_eq!(json["renders"][0]["background"], "transparent");
        assert_eq!(json["renders"][1]["background"], "#18181b");
    }
}
