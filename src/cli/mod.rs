//! Argument types only.
//!
//! No behaviour lives here: parsing is one concern and doing the work is
//! another, and keeping them apart is what lets the library be used without
//! the CLI.

use std::path::PathBuf;

pub mod doctor;
pub mod init;
pub mod inspect;
pub mod mcp;
pub mod render;
pub mod tui;

use clap::{Args, Parser, Subcommand, ValueEnum};

use shaipe::preview::{Backend, Scale};
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

    /// Force a preview backend instead of detecting one.
    ///
    /// Global, so that it works on the bare invocation as well as on `tui` and
    /// `doctor`. It is the documented escape hatch for a terminal whose
    /// graphics detection goes wrong, and `shaipe --preview blocks` — the
    /// obvious way to reach for it — was a parse error until it was.
    #[arg(long, global = true)]
    pub preview: Option<Backend>,

    /// Transmit previews at 1/N of the pane's resolution.
    ///
    /// Defaults to 2 under tmux and 1 otherwise. Kitty transmits raw pixels,
    /// and tmux passthrough is slow enough that a full-resolution preview
    /// takes seconds; the terminal scales the smaller image back up.
    #[arg(long, global = true, value_name = "N")]
    pub preview_scale: Option<Scale>,

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
    /// Report what Shaipe can work out about this terminal.
    Doctor(DoctorArgs),
    /// Serve the project's tools over MCP, for an agent to call.
    Mcp(McpArgs),
    /// Create a project from nothing.
    Init(InitArgs),
}

/// Arguments to `shaipe init`.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Where to create the project.
    #[arg(default_value = DEFAULT_PROJECT)]
    pub path: PathBuf,

    /// Overwrite the file if it already exists.
    #[arg(long)]
    pub force: bool,

    /// Seed the project's prompt — what it is meant to be, in your own words.
    #[arg(long)]
    pub prompt: Option<String>,

    /// Attach an existing image as the thing to reproduce, traced or
    /// vectorised — what an agent works from instead of, or alongside,
    /// `--prompt`. Recorded as a `source` reference; use `set_reference` to
    /// attach further references, or to change this one, once the project is
    /// open. The file must exist.
    #[arg(long)]
    pub source: Option<PathBuf>,
}

/// Arguments to `shaipe mcp`.
#[derive(Debug, Args)]
pub struct McpArgs {
    /// The project SVG to serve.
    #[arg(default_value = DEFAULT_PROJECT)]
    pub input: PathBuf,

    /// Pipe standard input and output to a running workspace.
    ///
    /// Not for people, which is why it is hidden. An ACP agent starts its MCP
    /// servers as subprocesses and speaks to them over stdio; a live workspace
    /// cannot be one, because its own stdio is the terminal. So it listens on
    /// a socket and has the agent start this, which is a byte pipe and nothing
    /// else. See ADR 010.
    #[arg(long, value_name = "ADDRESS", hide = true, conflicts_with = "input")]
    pub bridge: Option<String>,

    /// Save the project to disk after a tool changes it.
    ///
    /// Off by default. An agent silently rewriting a file in a working tree is
    /// how a tool stops being trusted, and `write_svg` says `saved: false` for
    /// the same reason.
    #[arg(long)]
    pub write: bool,
}

/// Arguments to `shaipe doctor`.
///
/// The backend comes from the global `--preview`, so there is nothing here
/// yet. Kept as a type so adding one is not a signature change.
#[derive(Debug, Args)]
pub struct DoctorArgs {}

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
///
/// No `--preview` here: it is global, so `shaipe tui --preview blocks` and
/// `shaipe --preview blocks` are the same flag and cannot disagree.
///
#[derive(Debug, Args)]
pub struct TuiArgs {
    /// The project SVG to open.
    ///
    /// Only on the subcommand. A top-level positional would make
    /// `shaipe render` ambiguous between a subcommand and a file name.
    #[arg(default_value = DEFAULT_PROJECT)]
    pub input: PathBuf,

    /// The ACP agent to drive, as a command line.
    ///
    /// Shaipe starts an agent you already have; it never hosts one, and it
    /// never sees your API key. Whatever model the agent is configured with is
    /// the model that answers.
    #[arg(long, env = "SHAIPE_AGENT", default_value = shaipe::acp::DEFAULT_AGENT)]
    pub agent: String,

    /// Open the workspace without an agent.
    #[arg(long, conflicts_with = "agent")]
    pub no_agent: bool,

    /// Approve the agent's own tools when it asks about them.
    ///
    /// Note *when it asks*: permission is resolved inside the agent, and it
    /// consults a client only about what its configuration marks as needing
    /// consent. OpenCode's defaults never ask, so with one this flag does very
    /// little.
    ///
    /// It does **not** re-enable the agent's file editing and shell, which
    /// Shaipe denies through the environment it starts the agent in, and which
    /// an explicit denial keeps denied whatever is auto-approved. See ADR 013.
    ///
    /// Shaipe's own tools are never refused, and never write to disk on their
    /// own: `write_svg` changes the project in memory until you press ctrl-s.
    #[arg(long)]
    pub yes: bool,
}

impl TuiArgs {
    /// What bare `shaipe` runs with.
    ///
    /// Produced by clap rather than written as a struct literal, so the
    /// defaults — the project name, the agent command, `SHAIPE_AGENT` — live
    /// only in the attributes above. A literal would restate them, and the
    /// copy that drifted would be the one nobody noticed.
    ///
    /// # Panics
    ///
    /// Never: every field has a default or is a flag, so there is nothing for
    /// the parse to reject.
    #[must_use]
    pub fn defaults() -> Self {
        use clap::FromArgMatches as _;

        let command = Self::augment_args(clap::Command::new("shaipe"));
        Self::from_arg_matches(&command.get_matches_from(["shaipe"]))
            .expect("every field of TuiArgs has a default")
    }
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

    #[test]
    fn init_defaults_to_the_conventional_project_name_and_no_force() {
        let Some(Command::Init(args)) = Cli::parse_from(["shaipe", "init"]).command else {
            panic!("expected an init command");
        };
        assert_eq!(args.path, PathBuf::from("logo.svg"));
        assert!(!args.force);
        assert!(args.prompt.is_none());
        assert!(args.source.is_none());
    }

    #[test]
    fn init_accepts_a_path_force_and_a_prompt() {
        let Some(Command::Init(args)) = Cli::parse_from([
            "shaipe",
            "init",
            "mark.svg",
            "--force",
            "--prompt",
            "A square and a bar.",
        ])
        .command
        else {
            panic!("expected an init command");
        };
        assert_eq!(args.path, PathBuf::from("mark.svg"));
        assert!(args.force);
        assert_eq!(args.prompt.as_deref(), Some("A square and a bar."));
    }

    #[test]
    fn init_accepts_a_source_image() {
        let Some(Command::Init(args)) =
            Cli::parse_from(["shaipe", "init", "mark.svg", "--source", "mockup.png"]).command
        else {
            panic!("expected an init command");
        };
        assert_eq!(args.path, PathBuf::from("mark.svg"));
        assert_eq!(args.source, Some(PathBuf::from("mockup.png")));
    }

    #[test]
    fn the_preview_backend_can_be_forced_without_naming_a_subcommand() {
        // `shaipe --preview blocks` was a parse error, which made the
        // documented escape hatch for broken graphics detection unusable from
        // the invocation people actually type.
        let cli = Cli::parse_from(["shaipe", "--preview", "blocks"]);
        assert_eq!(cli.preview, Some(Backend::Blocks));
        assert!(cli.command.is_none());
    }

    #[test]
    fn the_preview_backend_is_the_same_flag_on_every_subcommand() {
        for arguments in [
            vec!["shaipe", "--preview", "kitty"],
            vec!["shaipe", "tui", "--preview", "kitty"],
            vec!["shaipe", "--preview", "kitty", "tui"],
            vec!["shaipe", "doctor", "--preview", "kitty"],
        ] {
            let cli = Cli::try_parse_from(&arguments)
                .unwrap_or_else(|error| panic!("{arguments:?} should parse: {error}"));
            assert_eq!(cli.preview, Some(Backend::Kitty), "{arguments:?}");
        }
    }
}
