//! `shaipe mcp` — serve a project's tools to an agent.
//!
//! Two shapes, one command. Without `--bridge` it opens a project and serves
//! it over this process's stdio, which is the standalone server an external
//! MCP client configures. With `--bridge` it opens nothing and serves nothing:
//! it is a pipe to a workspace that is already running. See ADR 010.

use shaipe::Project;
use shaipe::mcp::Server;
use shaipe::tools::{Registry, SessionHandle};

use crate::cli::McpArgs;

/// Run `shaipe mcp`.
///
/// # Errors
///
/// Returns whatever opening the project or serving the transport returns.
pub async fn run(args: &McpArgs) -> shaipe::Result<()> {
    // Held for the whole session, and this is not optional. Standard output is
    // a JSON-RPC stream; a single `log::warn!` about an unresolved font in the
    // middle of it is a frame the client cannot parse, and the failure looks
    // like Shaipe speaking a broken protocol rather than like a warning. This
    // is the stdio twin of the alternate-screen rule in the workspace.
    let _quiet = shaipe::logging::suppress();

    // The bridge opens no project and serves no protocol: it is a pipe to a
    // workspace that already has both.
    if let Some(address) = &args.bridge {
        return shaipe::mcp::bridge::run(&address.parse()?).await;
    }

    let project = Project::open(&args.input)?;
    let session = SessionHandle::detached(project, Registry::new(), args.write);

    Server::new(session).serve_stdio().await
}
