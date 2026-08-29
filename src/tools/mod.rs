//! Operations an agent can perform on a project.
//!
//! # What this is, and what it is not
//!
//! Shaipe does not host a model, ship a provider or own a conversation. The
//! agent already exists — Codex, Claude Code, OpenCode, something else — and
//! what it lacks is the ability to *see* an SVG and to know what the project
//! around it says. This module is the vocabulary of those abilities.
//!
//! There is deliberately **no transport here**. [`crate::mcp`] serves this
//! registry over MCP and [`crate::acp`] drives an agent that calls it, and
//! neither is something this module knows about: a tool is a name, a schema
//! and a JSON-in/JSON-out call, and it stays that way so the CLI, the
//! workspace and a server all expose the same operations with the same
//! semantics.
//!
//! # Why a registry at all
//!
//! Because the alternative — an agent shelling out to `shaipe` and parsing
//! stdout — makes every tool's contract the accident of a print statement.
//! Naming the tools, their inputs and their outputs in one place means the
//! CLI, the MCP server and the TUI cannot drift into three dialects.
//!
//! # Mutation
//!
//! [`Tool::call`] receives the project by `&mut`, so a tool that changes
//! something needs no different trait. What it does *not* get is a way to save
//! it: writing to a working tree is a decision, not a side effect, and it is
//! taken by whoever owns the project — see [`Tool::mutates`].

mod builtin;
mod output;
pub mod session;

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::error::{Error, Result};
use crate::project::Project;

pub use output::{ToolImage, ToolOutput};
pub use session::{Applied, SessionCommand, SessionHandle};

/// An operation an agent can perform on a project.
pub trait Tool: Send + Sync {
    /// The name the agent calls it by. Stable; renaming one is a breaking
    /// change in the same way a diagnostic code is. See ADR 007.
    fn name(&self) -> &'static str;

    /// What it does, in a sentence. This is prompt text: it is the only thing
    /// the model reads before deciding whether to call it.
    fn description(&self) -> &'static str;

    /// A JSON Schema for the arguments object.
    ///
    /// Hand-written rather than derived. This is prompt text as much as the
    /// description is, and a derived schema carries Rust's vocabulary —
    /// `Option<u32>`, `nullable`, `format: uint32` — into a place a model
    /// reads. See ADR 008.
    fn input_schema(&self) -> Value;

    /// Whether calling it can change the project.
    ///
    /// Drives MCP's read-only hint, and tells a workspace whether a call means
    /// its preview is now stale.
    fn mutates(&self) -> bool {
        false
    }

    /// Perform it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidToolInput`] when the arguments do not make
    /// sense, and otherwise whatever the underlying operation returns.
    fn call(&self, project: &mut Project, input: &Value) -> Result<ToolOutput>;
}

/// Everything a transport needs to advertise one tool.
///
/// Owned rather than borrowed: an MCP server builds this once and then holds
/// it across an `await`, which a `&dyn Tool` into a registry owned by another
/// task cannot survive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescriptor {
    /// What the agent calls it.
    pub name: &'static str,
    /// What it does.
    pub description: &'static str,
    /// A JSON Schema for its arguments.
    pub input_schema: Value,
    /// Whether it can change the project.
    pub mutates: bool,
}

/// The tools available for a project.
pub struct Registry {
    tools: BTreeMap<&'static str, Box<dyn Tool>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// A registry holding no tools.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    /// A registry holding everything Shaipe implements.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self::empty();
        for tool in builtin::all() {
            registry.register(tool);
        }
        registry
    }

    /// Add a tool, replacing any tool of the same name.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    /// Every tool, in a stable order.
    ///
    /// Stable because this list becomes prompt text, and a set of tools that
    /// reorders itself between runs makes a model's behaviour irreproducible
    /// for no reason.
    pub fn tools(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.values().map(AsRef::as_ref)
    }

    /// The names of every registered tool.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().map(|name| (*name).to_owned()).collect()
    }

    /// Look a tool up.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(AsRef::as_ref)
    }

    /// Everything a transport needs to advertise the whole registry.
    #[must_use]
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools()
            .map(|tool| ToolDescriptor {
                name: tool.name(),
                description: tool.description(),
                input_schema: tool.input_schema(),
                mutates: tool.mutates(),
            })
            .collect()
    }

    /// Call a tool by name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownTool`], listing what does exist, and otherwise
    /// whatever the tool returns.
    pub fn call(&self, name: &str, project: &mut Project, input: &Value) -> Result<ToolOutput> {
        self.get(name)
            .ok_or_else(|| Error::UnknownTool {
                tool: name.to_owned(),
                known: self.names(),
            })?
            .call(project, input)
    }
}

/// An object schema with the given properties, of which some are required.
///
/// A helper rather than a literal at each call site so that every tool
/// advertises the same shape: `additionalProperties: false` throughout, so a
/// model that invents an argument is told rather than silently ignored.
pub(crate) fn object(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let properties: Map<String, Value> = properties
        .iter()
        .map(|(name, schema)| ((*name).to_owned(), schema.clone()))
        .collect();

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// A string argument.
pub(crate) fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

/// A positive integer argument. Every dimension Shaipe takes is one.
pub(crate) fn integer(description: &str) -> Value {
    json!({ "type": "integer", "minimum": 1, "description": description })
}

/// A bounded array of positive integers.
///
/// The bound is in the schema as well as in the tool's own check, so a model
/// that reads it never sends a list that will be refused.
pub(crate) fn integers(description: &str, max_items: usize) -> Value {
    json!({
        "type": "array",
        "items": { "type": "integer", "minimum": 1 },
        "maxItems": max_items,
        "description": description,
    })
}

/// Read a required string argument.
///
/// # Errors
///
/// Returns [`Error::InvalidToolInput`] naming the parameter, because a model
/// that gets an argument wrong can only correct itself if told which one.
pub(crate) fn required_str<'a>(tool: &str, input: &'a Value, name: &str) -> Result<&'a str> {
    input
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidToolInput {
            tool: tool.to_owned(),
            reason: format!("`{name}` is required and must be a string"),
        })
}

/// Read an optional positive integer, falling back when it is absent.
///
/// A zero or a negative is treated as absent rather than as an error: it can
/// only produce an empty render, which [`Error::InvalidSize`] would report
/// later and less clearly.
pub(crate) fn optional_u32(input: &Value, name: &str, fallback: u32) -> u32 {
    input
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;

    #[test]
    fn every_registered_tool_describes_itself() {
        // The description is prompt text. A tool with an empty one is a tool
        // the model will never choose correctly.
        for tool in Registry::new().tools() {
            assert!(!tool.name().is_empty());
            assert!(
                tool.description().len() > 20,
                "`{}` needs a description a model can act on",
                tool.name()
            );
        }
    }

    #[test]
    fn every_tool_publishes_a_schema_a_model_can_read() {
        // A schema is prompt text too. An argument with no description is one
        // the model has to guess the meaning of from its name alone.
        for tool in Registry::new().tools() {
            let schema = tool.input_schema();
            let name = tool.name();

            assert_eq!(schema["type"], "object", "`{name}` must take an object");
            assert_eq!(
                schema["additionalProperties"], false,
                "`{name}` must reject arguments it does not understand, so a \
                 model that invents one is told rather than ignored"
            );

            let properties = schema["properties"].as_object().unwrap_or_else(|| {
                panic!("`{name}` must list its properties, even if there are none")
            });

            for (argument, description) in properties {
                assert!(
                    description["description"]
                        .as_str()
                        .is_some_and(|text| text.len() > 10),
                    "`{name}.{argument}` needs a description a model can act on"
                );
            }

            // Anything required must actually be described, or the model is
            // told to send an argument it has never been told the meaning of.
            for required in schema["required"].as_array().into_iter().flatten() {
                let required = required.as_str().expect("a required name is a string");
                assert!(
                    properties.contains_key(required),
                    "`{name}` requires `{required}` without describing it"
                );
            }
        }
    }

    #[test]
    fn the_tool_list_is_in_a_stable_order() {
        assert_eq!(Registry::new().names(), Registry::new().names());
    }

    #[test]
    fn the_tool_names_are_the_ones_the_adr_records() {
        // A tool name is a public interface: it appears in a model's context,
        // in a user's config and in whatever conversation history an agent
        // keeps. Renaming one is a breaking change, so it should take a
        // deliberate edit here rather than happening as a side effect of a
        // refactor. See ADR 007.
        assert_eq!(
            Registry::new().names(),
            [
                "get_palette",
                "get_project",
                "get_reference_image",
                "get_references",
                "get_svg",
                "get_variants",
                "render_grid",
                "render_svg",
                "set_generation",
                "set_palette_colour",
                "set_reference",
                "write_svg",
                "write_variant",
            ]
        );
    }

    #[test]
    fn calling_a_tool_that_does_not_exist_lists_the_ones_that_do() {
        let mut project = fixtures::project();
        let error = Registry::new()
            .call("vectorise", &mut project, &Value::Null)
            .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("vectorise"), "{rendered}");
        assert!(matches!(error, Error::UnknownTool { .. }));
    }

    #[test]
    fn a_registered_tool_replaces_one_of_the_same_name() {
        // How a host adds a capability Shaipe does not have, or overrides one
        // it does.
        struct Stub;
        impl Tool for Stub {
            fn name(&self) -> &'static str {
                "get_project"
            }
            fn description(&self) -> &'static str {
                "a replacement, for testing that registration overrides"
            }
            fn input_schema(&self) -> Value {
                object(&[], &[])
            }
            fn call(&self, _: &mut Project, _: &Value) -> Result<ToolOutput> {
                Ok(ToolOutput::json(Value::String("stub".to_owned())))
            }
        }

        let mut registry = Registry::new();
        let before = registry.names().len();
        registry.register(Box::new(Stub));

        assert_eq!(registry.names().len(), before);
        assert_eq!(
            registry
                .call("get_project", &mut fixtures::project(), &Value::Null)
                .unwrap()
                .value,
            Value::String("stub".to_owned())
        );
    }

    #[test]
    fn a_descriptor_says_the_same_thing_the_tool_does() {
        // The descriptor is what a transport advertises. If it can disagree
        // with the tool, a model is told about arguments that do not exist.
        let registry = Registry::new();
        for (descriptor, tool) in registry.descriptors().iter().zip(registry.tools()) {
            assert_eq!(descriptor.name, tool.name());
            assert_eq!(descriptor.description, tool.description());
            assert_eq!(descriptor.input_schema, tool.input_schema());
            assert_eq!(descriptor.mutates, tool.mutates());
        }
    }

    #[test]
    fn an_optional_dimension_falls_back_rather_than_rendering_nothing() {
        // Zero is the interesting case: it is a number, so a naive read
        // accepts it, and it can only produce an empty image.
        assert_eq!(optional_u32(&json!({ "width": 64 }), "width", 512), 64);
        assert_eq!(optional_u32(&json!({ "width": 0 }), "width", 512), 512);
        assert_eq!(optional_u32(&json!({}), "width", 512), 512);
    }
}
