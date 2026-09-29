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

/// A boolean argument.
pub(crate) fn boolean(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
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
/// Every backticked word in `text` that has the shape of a tool name.
///
/// By the closed verb set of ADR 007, so that `source` or `<image>` in the
/// same prose are not mistaken for tools. Shared by every module whose prompt
/// text names tools, so each is checked against the registry the same way.
pub(crate) fn tool_names_in(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|word| {
            ["get_", "render_", "write_", "set_", "compare_"]
                .iter()
                .any(|verb| word.starts_with(verb))
        })
        .map(str::to_owned)
        .collect()
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
        // Sorted, not merely equal to itself: a list that reorders between
        // runs becomes prompt text that does too, and alphabetical is the one
        // order nobody has to remember.
        let names = Registry::new().names();
        let mut sorted = names.clone();
        sorted.sort();

        assert_eq!(names, sorted);
        assert_eq!(names, Registry::new().names());
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
                "compare_reference",
                "get_palette",
                "get_project",
                "get_reference_analysis",
                "get_reference_image",
                "get_reference_trace",
                "get_references",
                "get_svg",
                "get_variants",
                "get_workflow",
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
    fn every_tool_the_instructions_name_exists() {
        // The instructions are prompt text that names tools. One renamed
        // without them would send an agent to call something that is not there.
        let names = Registry::new().names();
        let mentioned = tool_names_in(&crate::workflow::instructions::reconstruction());

        assert!(
            !mentioned.is_empty(),
            "the instructions name no tool at all"
        );
        for name in mentioned {
            assert!(
                names.contains(&name),
                "the instructions name `{name}`, which is not a tool"
            );
        }
    }

    #[test]
    fn the_instructions_name_every_tool_the_reconstruction_loop_uses() {
        let mentioned = tool_names_in(&crate::workflow::instructions::reconstruction());

        for name in [
            "get_references",
            "get_reference_image",
            "get_reference_analysis",
            "get_reference_trace",
            "compare_reference",
            "get_workflow",
            "render_svg",
            "write_svg",
            "write_variant",
        ] {
            assert!(
                mentioned.iter().any(|mentioned| mentioned == name),
                "the loop needs `{name}` and the instructions never mention it"
            );
        }
    }

    #[test]
    fn the_instructions_and_get_workflows_description_state_the_same_phase_order() {
        // Two places tell a model the order of the phases. Each is generated
        // from or checked against `WorkflowKind::phases`, so they cannot
        // diverge without this failing.
        use crate::workflow::WorkflowKind;
        use crate::workflow::instructions::{reconstruction, sequence};

        let registry = Registry::new();
        let description = registry
            .tools()
            .find(|tool| tool.name() == "get_workflow")
            .expect("get_workflow is registered")
            .description();
        let instructions = reconstruction();

        for kind in [WorkflowKind::FromScratch, WorkflowKind::Reference] {
            let order = sequence(kind);
            assert!(
                description.contains(&order),
                "get_workflow disagrees about {kind}: {order}"
            );
            assert!(
                instructions.contains(&order),
                "the instructions disagree about {kind}: {order}"
            );
        }
    }

    /// Everything a model reads about a tool before calling it: its
    /// description, then each argument's.
    fn prompt_text(tool: &dyn Tool) -> Vec<(String, String)> {
        let mut texts = vec![(tool.name().to_owned(), tool.description().to_owned())];

        let schema = tool.input_schema();
        for (argument, property) in schema["properties"].as_object().into_iter().flatten() {
            texts.push((
                format!("{}.{argument}", tool.name()),
                property["description"].as_str().unwrap_or("").to_owned(),
            ));
        }
        texts
    }

    #[test]
    fn every_tool_a_contract_names_exists() {
        // Descriptions and argument text name other tools ("call
        // `get_reference_image` next"). The instructions are checked for this
        // already; the contracts are the text a model reads on every listing,
        // and a tool renamed without them would send it to call nothing.
        let registry = Registry::new();
        let names = registry.names();

        let mut checked = 0;
        for tool in registry.tools() {
            for (place, text) in prompt_text(tool) {
                for mentioned in tool_names_in(&text) {
                    checked += 1;
                    assert!(
                        names.contains(&mentioned),
                        "`{place}` names `{mentioned}`, which is not a tool"
                    );
                }
            }
        }
        assert!(
            checked > 20,
            "only {checked} tool names were found to check"
        );
    }

    #[test]
    fn the_reconstruction_loop_is_followable_from_the_contracts_alone() {
        // Each contract names the tool a model should reach for next. Without
        // these links a description is an island, and the loop — look, measure,
        // construct, render, compare, fix — exists only in the instructions,
        // which not every agent reads. Kept as a table so the loop can be read
        // here as well as followed.
        let registry = Registry::new();

        for (tool, next) in [
            ("get_references", &["get_reference_image"][..]),
            (
                "get_reference_analysis",
                &["get_reference_image", "get_reference_trace"],
            ),
            (
                "get_reference_trace",
                &["get_reference_analysis", "write_variant", "render_svg"],
            ),
            (
                "get_workflow",
                &[
                    "get_reference_analysis",
                    "write_variant",
                    "render_svg",
                    "compare_reference",
                ],
            ),
            ("get_svg", &["write_variant", "write_svg"]),
            (
                "write_variant",
                &["write_svg", "render_svg", "compare_reference"],
            ),
            (
                "write_svg",
                &["write_variant", "render_svg", "compare_reference"],
            ),
            ("render_svg", &["compare_reference", "render_grid"]),
            (
                "compare_reference",
                &["write_variant", "write_svg", "render_svg"],
            ),
            ("set_reference", &["get_reference_image"]),
        ] {
            let description = registry
                .get(tool)
                .unwrap_or_else(|| panic!("`{tool}` is not a tool"))
                .description();
            let mentioned = tool_names_in(description);

            for name in next {
                assert!(
                    mentioned.iter().any(|mentioned| mentioned == name),
                    "`{tool}` never points at `{name}`, so the loop breaks there"
                );
            }
        }
    }

    #[test]
    fn no_contract_but_the_workflows_restates_the_phase_order() {
        // The order of the phases has one home in prose — `get_workflow`, whose
        // description is checked against the instructions — and one at run
        // time. A second copy in another description is a second place to
        // forget to update.
        use crate::workflow::WorkflowKind;
        use crate::workflow::instructions::sequence;

        for tool in Registry::new()
            .tools()
            .filter(|t| t.name() != "get_workflow")
        {
            for (place, text) in prompt_text(tool) {
                for kind in [WorkflowKind::FromScratch, WorkflowKind::Reference] {
                    assert!(
                        !text.contains(&sequence(kind)),
                        "`{place}` repeats the {kind} phase order; point at `get_workflow`"
                    );
                }
            }
        }
    }

    #[test]
    fn no_contract_claims_to_be_the_only_way_or_the_only_first_step() {
        // Both were said, both were false — `get_variants` lists the variant
        // names `get_project` claimed to be the only way to learn, and
        // `render_grid` and `compare_reference` also show the artwork — and a
        // model that believes one skips the tool that would have helped.
        for tool in Registry::new().tools() {
            for (place, text) in prompt_text(tool) {
                let text = text.to_lowercase();
                for phrase in ["only way", "call this first"] {
                    assert!(
                        !text.contains(phrase),
                        "`{place}` says \"{phrase}\", which overclaims"
                    );
                }
            }
        }
    }

    #[test]
    fn a_src_or_variant_argument_says_which_tool_lists_its_values() {
        // The value is one the model cannot invent: an attached path, or a
        // declared name. Saying where to get it is the difference between
        // one call and a refusal followed by another.
        for tool in Registry::new().tools() {
            let schema = tool.input_schema();
            for (argument, listing) in [("src", "get_references"), ("variant", "get_variants")] {
                let Some(property) = schema["properties"].get(argument) else {
                    continue;
                };
                // `set_reference` takes a `src` that is new by design.
                if tool.name() == "set_reference" {
                    continue;
                }
                assert!(
                    property["description"]
                        .as_str()
                        .is_some_and(|text| text.contains(listing)),
                    "`{}.{argument}` never says the value comes from `{listing}`",
                    tool.name()
                );
            }
        }
    }

    #[test]
    fn a_description_stays_short_enough_to_be_read_on_every_listing() {
        // Every description is in the model's context on every turn, for every
        // tool. The bound is set just above the longest today, so growth has
        // to be a decision made here rather than an accretion.
        for tool in Registry::new().tools() {
            let length = tool.description().len();
            assert!(
                length <= 1_200,
                "`{}` is {length} characters; move guidance that is not about \
                 this tool into the instructions or an argument",
                tool.name()
            );
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
