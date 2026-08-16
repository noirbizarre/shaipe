//! Operations an agent can perform on a project.
//!
//! # What this is, and what it is not
//!
//! Shaipe does not host a model, ship a provider or own a conversation. The
//! agent already exists — Codex, Claude Code, OpenCode, something else — and
//! what it lacks is the ability to *see* an SVG and to know what the project
//! around it says. This module is the vocabulary of those abilities.
//!
//! There is deliberately **no transport here**. An MCP server, a JSON-RPC
//! endpoint or a subprocess protocol are all things that would call
//! [`Registry::call`]; none of them are things this crate has to be. Adding
//! one is additive, and building one before there is anything to serve would
//! be inventing requirements.
//!
//! # Why a registry at all
//!
//! Because the alternative — an agent shelling out to `shaipe` and parsing
//! stdout — makes every tool's contract the accident of a print statement.
//! Naming the tools, their inputs and their outputs in one place means the
//! CLI, a future MCP server and the TUI expose the same operations with the
//! same semantics.
//!
//! Everything here is read-only for now. Mutating tools are the natural next
//! set — editing the palette, writing a variant, recording a generation — and
//! the trait is shaped to take them: [`Tool::call`] receives the project by
//! `&mut`, so a tool that changes something does not need a different trait.

mod builtin;

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::project::Project;

/// A description of what a tool accepts.
///
/// Not a JSON Schema type, deliberately: a transport that needs one can build
/// it from this, and the alternative is a schema dependency in a crate whose
/// job is drawing logos. What every transport actually needs is a name, a
/// sentence and a list of parameters, and that is what this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    /// The key in the input object.
    pub name: &'static str,
    /// What it means, in a sentence a model can act on.
    pub description: &'static str,
    /// Whether omitting it is an error.
    pub required: bool,
}

/// An operation an agent can perform on a project.
pub trait Tool {
    /// The name the agent calls it by. Stable; renaming one is a breaking
    /// change in the same way a diagnostic code is.
    fn name(&self) -> &'static str;

    /// What it does, in a sentence. This is prompt text: it is the only thing
    /// the model reads before deciding whether to call it.
    fn description(&self) -> &'static str;

    /// What it accepts.
    fn parameters(&self) -> &'static [Parameter];

    /// Perform it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidToolInput`] when the arguments do not make
    /// sense, and otherwise whatever the underlying operation returns.
    fn call(&self, project: &mut Project, input: &Value) -> Result<Value>;
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

    /// Call a tool by name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownTool`], listing what does exist, and otherwise
    /// whatever the tool returns.
    pub fn call(&self, name: &str, project: &mut Project, input: &Value) -> Result<Value> {
        self.get(name)
            .ok_or_else(|| Error::UnknownTool {
                tool: name.to_owned(),
                known: self.names(),
            })?
            .call(project, input)
    }
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
    fn the_tool_list_is_in_a_stable_order() {
        assert_eq!(Registry::new().names(), Registry::new().names());
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
                "inspect_project"
            }
            fn description(&self) -> &'static str {
                "a replacement, for testing that registration overrides"
            }
            fn parameters(&self) -> &'static [Parameter] {
                &[]
            }
            fn call(&self, _: &mut Project, _: &Value) -> Result<Value> {
                Ok(Value::String("stub".to_owned()))
            }
        }

        let mut registry = Registry::new();
        let before = registry.names().len();
        registry.register(Box::new(Stub));

        assert_eq!(registry.names().len(), before);
        assert_eq!(
            registry
                .call("inspect_project", &mut fixtures::project(), &Value::Null)
                .unwrap(),
            Value::String("stub".to_owned())
        );
    }
}
