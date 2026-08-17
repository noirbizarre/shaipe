//! The shaipe binary.

#![allow(clippy::result_large_err)]

use std::io::Write as _;
use std::process::ExitCode;

use clap::Parser;

mod cli;

use cli::{Cli, Command, TuiArgs};

#[tokio::main]
async fn main() -> ExitCode {
    let args = Cli::parse();
    shaipe::logging::init(args.verbose);

    // Global, so every path reads the same flag.
    let preview = args.preview.unwrap_or_default();
    let args_verbose = args.verbose;
    let scale = args.preview_scale;

    let result = match &args.command {
        Some(Command::Render(args)) => write_lines(|out| cli::render::run(args, out)),
        Some(Command::Inspect(args)) => write_lines(|out| cli::inspect::run(args, out)),
        // The report is built *before* the lock is taken: producing it queries
        // the terminal from another thread, which the lock would block.
        Some(Command::Doctor(args)) => {
            let report = cli::doctor::report(args, preview, scale);
            write_lines(|out| {
                out.write_all(report.as_bytes())
                    .map_err(|source| shaipe::Error::io("stdout", source))
            })
        }
        Some(Command::Tui(args)) => cli::tui::run(args, preview, scale, args_verbose).await,
        Some(Command::Mcp(args)) => cli::mcp::run(args).await,
        // Bare `shaipe` opens the workspace on the conventional project, which
        // is the thing a person in a project directory almost always wants.
        None => {
            cli::tui::run(
                &TuiArgs {
                    input: cli::DEFAULT_PROJECT.into(),
                },
                preview,
                scale,
                args_verbose,
            )
            .await
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Rendered here rather than returned as `miette::Result`, so the
            // exit code is chosen explicitly instead of inherited from
            // whatever the report wrapper decides.
            eprintln!("{:?}", miette::Report::new(error));
            ExitCode::FAILURE
        }
    }
}

/// Run a command that writes a stream of lines to standard output.
///
/// The lock is taken here and released before this returns, which is the whole
/// point of the function.
///
/// Deliberately **not** `async`, and the closure it takes deliberately cannot
/// `await`. Holding a `StdoutLock` across an await point would let the runtime
/// schedule another task onto the same thread while the lock is held, which is
/// the same failure as holding it across the workspace and is harder to see.
///
/// It must **never** be held across [`cli::tui::run`]. `Stdout` is guarded by a
/// re-entrant lock: re-entrant for the thread that holds it, blocking for every
/// other one. `ratatui-image` queries the terminal's graphics capabilities from
/// a spawned thread, so a lock held here would block that thread until its
/// timeout expired, the query would never be written, and the workspace would
/// silently fall back to half-blocks on a terminal that supports Kitty.
///
/// That is not hypothetical: it is the bug this function exists to prevent, and
/// the `stdout-lock-not-held-across-the-tui` hook in `prek.toml` keeps it that
/// way.
fn write_lines(
    command: impl FnOnce(&mut dyn std::io::Write) -> shaipe::Result<()>,
) -> shaipe::Result<()> {
    // One expression, and the only `.lock()` in the crate, so the guard hook
    // can find it by grep. `StdoutLock<'static>` makes the temporary sound.
    let mut out = std::io::stdout().lock();

    let result = command(&mut out);

    // Flushed before the result is returned, so a write that fails at the very
    // end is reported rather than silently dropped.
    result.and_then(|()| {
        out.flush()
            .map_err(|source| shaipe::Error::io("stdout", source))
    })
}
