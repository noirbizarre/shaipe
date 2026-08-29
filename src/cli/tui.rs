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
    // A path that does not exist yet behaves as if `shaipe init` had been run
    // on it: the workspace opens on a fresh, unsaved project rather than
    // failing. Nothing is written until the project is saved.
    let existed = args.input.exists();
    let project = if existed {
        Project::open(&args.input)?
    } else {
        Project::init(&args.input)?
    };

    // Resolved here, and a failure to resolve is *not* fatal: the reason
    // travels into the workspace and is shown in the prompt pane, where the
    // person who cannot send a prompt is looking. Somebody without an agent
    // installed must still be able to open and read their own project — the
    // same rule as a preview backend that will not start.
    let agent = if args.no_agent {
        AgentChoice::None
    } else {
        match AgentChoice::discover(&args.agent, &project) {
            AgentChoice::Start(config) => {
                let config = if args.yes {
                    config.with_policy(Policy::AllowAll)
                } else {
                    *config
                };
                let config = match &args.model {
                    Some(model) => config.with_model(model.clone()),
                    None => config,
                };
                AgentChoice::Start(Box::new(config))
            }
            choice => choice,
        }
    };

    shaipe::tui::run(project, !existed, preview, scale, verbose, agent).await
}
