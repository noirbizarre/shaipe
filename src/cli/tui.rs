//! `shaipe tui`.
//!
//! A thin adapter: open the project, hand it to the workspace. Everything that
//! makes the workspace work lives in the library, so the TUI is testable
//! without a terminal and usable without the binary.

use shaipe::acp::{AgentChoice, Policy};
use shaipe::preview::{Backend, Scale};
use shaipe::{Project, Result};

use crate::cli::TuiArgs;

/// Run `shaipe tui`.
///
/// # Errors
///
/// Returns whatever opening the project or running the workspace returns.
pub async fn run(
    args: &TuiArgs,
    preview: Backend,
    scale: Option<Scale>,
    verbose: u8,
) -> Result<()> {
    let project = Project::open(&args.input)?;

    // Resolved here, and a failure to resolve is *not* fatal: the reason is
    // carried into the workspace and shown in the prompt pane. Somebody
    // without OpenCode installed must still be able to open and read their own
    // project — the same rule as a preview backend that will not start.
    let agent = if args.no_agent {
        AgentChoice::None
    } else {
        match AgentChoice::discover(&args.agent, &project) {
            AgentChoice::Start(config) if args.yes => {
                AgentChoice::Start(Box::new(config.with_policy(Policy::AllowAll)))
            }
            choice => choice,
        }
    };

    shaipe::tui::run(project, preview, scale, verbose, agent).await
}
