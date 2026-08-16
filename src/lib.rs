//! An LLM-native SVG asset workspace
//!
//! The engine lives here; `src/main.rs` is a thin CLI over it. That split is
//! what lets the integration tests assert against the library for behaviour
//! and against the binary for output.

// miette's `Diagnostic` payloads carry source text and spans, which puts most
// error variants past clippy's 128-byte `Result` threshold. The lint is right
// about the cost and wrong about the trade: a large error that says what to do
// beats a small one that does not.
#![allow(clippy::result_large_err)]
#![warn(missing_docs)]

pub mod error;
pub mod inspect;
pub mod logging;
pub mod preview;
pub mod project;
pub mod render;
pub mod tools;
pub mod tui;

#[cfg(test)]
pub(crate) mod fixtures;

pub use error::{Error, Result};
pub use project::Project;
pub use render::{RenderedAsset, Renderer, render};

/// Run the thing.
///
/// # Errors
///
/// Returns [`Error`] when there is nothing to do.
pub fn run() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_with_nothing_to_do_succeeds() {
        assert!(run().is_ok());
    }
}
