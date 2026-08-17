//! Serving Shaipe's tools over the Model Context Protocol.
//!
//! A thin adapter, deliberately. Everything an agent can do lives in
//! [`crate::tools`]; this module knows how to say it in MCP and nothing else.
//! The test of the boundary is that `src/mcp/` contains no logic about
//! projects, rendering or SVG — only about content blocks, schemas and
//! transports.
//!
//! # Two modes, one implementation
//!
//! ```text
//!   an external client                    the workspace
//!          │                                    │
//!          │ stdio                              │ owns the live Project
//!          ▼                                    ▼
//!    `shaipe mcp <project>`              Listener on a socket
//!          │                                    ▲
//!          │                                    │ `shaipe mcp --bridge`
//!          └────────► SessionHandle ◄───────────┘
//!                          │
//!                   tools::Registry
//! ```
//!
//! They share [`Server`] and everything under it, and share no lifecycle at
//! all: one opens a file and owns it, the other reaches a project someone else
//! is looking at. See ADR 010.

pub mod bridge;
pub mod listener;
mod server;

pub use listener::{Address, Listener};
pub use server::Server;
