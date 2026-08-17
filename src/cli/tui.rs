//! `shaipe tui`.
//!
//! A thin adapter: open the project, hand it to the workspace. Everything that
//! makes the workspace work lives in the library, so the TUI is testable
//! without a terminal and usable without the binary.

use shaipe::preview::{Backend, Scale};
use shaipe::{Project, Result};

use crate::cli::TuiArgs;

/// Run `shaipe tui`.
///
/// # Errors
///
/// Returns whatever opening the project or running the workspace returns.
pub fn run(args: &TuiArgs, preview: Backend, scale: Option<Scale>, verbose: u8) -> Result<()> {
    let project = Project::open(&args.input)?;
    shaipe::tui::run(project, preview, scale, verbose)
}
