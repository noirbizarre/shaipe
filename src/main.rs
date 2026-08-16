//! The shaipe binary.

#![allow(clippy::result_large_err)]

use std::io::Write as _;
use std::process::ExitCode;

use clap::Parser;

mod cli;

use cli::{Cli, Command, TuiArgs};

fn main() -> ExitCode {
    let args = Cli::parse();
    shaipe::logging::init(args.verbose);

    // Locked once for the whole run: every command writes a stream of lines,
    // and re-locking per line would interleave with a warning from `log`.
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let result = match &args.command {
        Some(Command::Render(args)) => cli::render::run(args, &mut out),
        Some(Command::Inspect(args)) => cli::inspect::run(args, &mut out),
        Some(Command::Tui(args)) => cli::tui::run(args),
        // Bare `shaipe` opens the workspace on the conventional project, which
        // is the thing a person in a project directory almost always wants.
        None => cli::tui::run(&TuiArgs {
            input: cli::DEFAULT_PROJECT.into(),
            preview: None,
        }),
    };

    // Flushed before the exit code is decided, so a write that fails at the
    // very end is reported rather than silently dropped.
    let flushed = out.flush();

    match result
        .map_err(Some)
        .and_then(|()| flushed.map_err(|_| None))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Rendered here rather than returned as `miette::Result`, so the
            // exit code is chosen explicitly instead of inherited from
            // whatever the report wrapper decides.
            if let Some(error) = error {
                eprintln!("{:?}", miette::Report::new(error));
            }
            ExitCode::FAILURE
        }
    }
}
