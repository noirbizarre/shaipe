//! `shaipe init`.
//!
//! Creates a project from nothing: a minimal placeholder icon, declared as
//! the primary variant, ready to open in the workspace or hand to an agent.

use std::io::Write;

use shaipe::{Error, Project, Result};

use crate::cli::InitArgs;

/// Run `shaipe init`.
///
/// # Errors
///
/// Returns [`Error::ProjectExists`] if the file is already there and
/// `--force` was not given, or whatever creating it returns.
pub fn run(args: &InitArgs, out: &mut dyn Write) -> Result<()> {
    if args.path.exists() && !args.force {
        return Err(Error::ProjectExists {
            path: args.path.clone(),
        });
    }

    let mut project = Project::init(&args.path)?;
    if let Some(prompt) = &args.prompt {
        project.metadata_mut().prompt = Some(prompt.clone());
    }
    project.save()?;

    writeln!(out, "created {}", args.path.display()).map_err(|source| Error::io(&args.path, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::InitArgs;

    fn args(path: impl Into<std::path::PathBuf>) -> InitArgs {
        InitArgs {
            path: path.into(),
            force: false,
            prompt: None,
        }
    }

    #[test]
    fn creating_a_project_writes_a_file_that_reopens_successfully() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");

        let mut out = Vec::new();
        run(&args(&path), &mut out).unwrap();

        assert!(path.is_file());
        let project = Project::open(&path).unwrap();
        assert_eq!(project.metadata().primary.as_deref(), Some("icon"));

        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains(&path.display().to_string()), "{printed}");
    }

    #[test]
    fn creating_a_project_where_one_already_exists_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, "not touched").unwrap();

        let error = run(&args(&path), &mut Vec::new()).unwrap_err();
        assert!(matches!(error, Error::ProjectExists { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not touched");
    }

    #[test]
    fn force_overwrites_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, "not touched").unwrap();

        let mut forced = args(&path);
        forced.force = true;
        run(&forced, &mut Vec::new()).unwrap();

        assert!(Project::open(&path).is_ok());
    }

    #[test]
    fn a_prompt_ends_up_in_the_saved_project() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");

        let mut prompted = args(&path);
        prompted.prompt = Some("A square and a bar.".to_owned());
        run(&prompted, &mut Vec::new()).unwrap();

        let project = Project::open(&path).unwrap();
        assert_eq!(
            project.metadata().prompt.as_deref(),
            Some("A square and a bar.")
        );
    }
}
