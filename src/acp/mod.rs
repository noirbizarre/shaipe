//! Driving an agent over the Agent Client Protocol.
//!
//! Shaipe is an ACP **client**. It does not host a model, hold a key or own a
//! conversation — it starts an agent the user already installed and configured,
//! and hands that agent Shaipe's own tools to call back into. See ADR 011,
//! which argues that this does not contradict ADR 004.
//!
//! ```text
//! workspace ──ACP──> opencode acp ──> whatever model the user configured
//!     ▲                    │
//!     └────────MCP─────────┘
//! ```
//!
//! Nothing outside this module sees an ACP type. The protocol's own update
//! enum is `#[non_exhaustive]`; it is translated once, into
//! [`AgentUpdate`], so that the protocol growing cannot reach the panes.

mod agent;
mod update;

pub use agent::{Agent, AgentChoice, AgentConfig, DEFAULT_AGENT, McpServerSpec, Policy};
pub use update::{AgentUpdate, ToolStatus};
