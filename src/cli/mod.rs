//! Argument types only.
//!
//! No behaviour lives here: parsing is one concern and doing the work is
//! another, and keeping them apart is what lets the library be used without
//! the CLI.

use std::path::PathBuf;

pub mod inspect;
pub mod render;
pub mod tui;

use clap::{Args, Parser, Subcommand, ValueEnum};

use shaipe::preview::Backend;
use shaipe::project::Format;

/// Where a project is looked for when the command line does not say.
///
/// The dogfooding convention — a project's own `logo.svg` at the root of its
/// repository — made into a default, so that `shaipe` on its own does the
/// obvious thing inside a project.
pub const DEFAULT_PROJECT: &str = "logo.svg";

/// An LLM-native SVG asset workspace
#[derive(Debug, Parser)]
#[command(name = "shaipe", version, about, long_about = None)]
pub struct Cli {
    /// Increase verbosity. Repeat for more.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// The subcommand to run. Defaults to opening the workspace.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Render a project's assets. Never calls out to a network or a model.
    Render(RenderArgs),
    /// Describe what a project contains.
    Inspect(InspectArgs),
    /// Open a project in the interactive workspace.
    Tui(TuiArgs),
}

/// Arguments to `shaipe render`.
#[derive(Debug, Args)]
pub struct RenderArgs {
    /// The project SVG to render.
    #[arg(default_value = DEFAULT_PROJECT)]
    pub input: PathBuf,

    /// Where to write the assets. Created if it does not exist.
    #[arg(short, long, default_value = "dist")]
    pub output: PathBuf,

    /// Render only this declared specification. Repeatable.
    #[arg(short, long, conflicts_with_all = ["variant", "width", "height", "format", "background"])]
    pub spec: Vec<String>,

    /// Render this variant instead of the project's declared specifications.
    #[arg(long, requires = "width")]
    pub variant: Option<String>,

    /// Canvas width, in pixels.
    #[arg(long, requires = "variant")]
    pub width: Option<u32>,

    /// Canvas height, in pixels. Defaults to the width, making a square.
    #[arg(long, requires = "variant")]
    pub height: Option<u32>,

    /// How to encode the ad-hoc render.
    #[arg(long, requires = "variant")]
    pub format: Option<Format>,

    /// What to draw the ad-hoc render on: `transparent` or a CSS hex colour.
    #[arg(long, requires = "variant")]
    pub background: Option<String>,

    /// Name the ad-hoc render's output file. Defaults to the variant's name.
    #[arg(long, requires = "variant")]
    pub name: Option<String>,

    /// Refuse to fall back to system fonts.
    ///
    /// A system font makes the render depend on the machine that produced it,
    /// which is exactly what a CI check must not tolerate.
    #[arg(long)]
    pub strict_fonts: bool,

    /// Report what would be written without writing it.
    #[arg(long)]
    pub dry_run: bool,
}

/// How to present a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum ReportFormat {
    /// For a person.
    #[default]
    Text,
    /// For a program. The surface an agent should consume.
    Json,
}

/// Arguments to `shaipe inspect`.
#[derive(Debug, Args)]
pub struct InspectArgs {
    /// The project SVG to describe.
    #[arg(default_value = DEFAULT_PROJECT)]
    pub input: PathBuf,

    /// How to present the report.
    #[arg(short, long, value_enum, default_value_t)]
    pub format: ReportFormat,
}

/// Arguments to `shaipe tui`.
#[derive(Debug, Args)]
pub struct TuiArgs {
    /// The project SVG to open.
    #[arg(default_value = DEFAULT_PROJECT)]
    pub input: PathBuf,

    /// Force a preview backend instead of detecting one.
    #[arg(long)]
    pub preview: Option<Backend>,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    // Catches the derive mistakes that would otherwise only surface as a panic
    // the first time a user runs the binary.
    #[test]
    fn the_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn render_defaults_to_the_conventional_project_and_output_directory() {
        let Some(Command::Render(args)) = Cli::parse_from(["shaipe", "render"]).command else {
            panic!("expected a render command");
        };
        assert_eq!(args.input, PathBuf::from("logo.svg"));
        assert_eq!(args.output, PathBuf::from("dist"));
        assert!(args.spec.is_empty());
        assert!(args.variant.is_none());
    }

    #[test]
    fn an_ad_hoc_render_requires_both_a_variant_and_a_width() {
        // Half an ad-hoc specification is not a specification, and guessing
        // the missing half would produce an asset nobody asked for.
        assert!(Cli::try_parse_from(["shaipe", "render", "--variant", "icon"]).is_err());
        assert!(Cli::try_parse_from(["shaipe", "render", "--width", "64"]).is_err());
        assert!(
            Cli::try_parse_from(["shaipe", "render", "--variant", "icon", "--width", "64"]).is_ok()
        );
    }

    #[test]
    fn naming_a_spec_and_describing_an_ad_hoc_render_are_mutually_exclusive() {
        // They answer the same question — what to render — and accepting both
        // would mean silently ignoring one of them.
        assert!(
            Cli::try_parse_from([
                "shaipe",
                "render",
                "--spec",
                "favicon-32",
                "--variant",
                "icon",
                "--width",
                "64",
            ])
            .is_err()
        );
    }

    #[test]
    fn running_shaipe_with_no_subcommand_is_allowed() {
        assert!(Cli::parse_from(["shaipe"]).command.is_none());
    }
}
