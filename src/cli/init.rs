//! `shaipe init`.
//!
//! Creates a project from nothing: a minimal placeholder icon, declared as
//! the primary variant, ready to open in the workspace or hand to an agent.

use std::io::Write;

use shaipe::project::{Reference, ReferenceKind};
use shaipe::{Error, Project, Result};

use crate::cli::InitArgs;

/// Run `shaipe init`.
///
/// # Errors
///
/// Returns [`Error::ProjectExists`] if the file is already there and
/// `--force` was not given, [`Error::Io`] if `--source` names a file that
/// does not exist, or whatever creating it returns.
pub fn run(args: &InitArgs, out: &mut dyn Write) -> Result<()> {
    if args.path.exists() && !args.force {
        return Err(Error::ProjectExists {
            path: args.path.clone(),
        });
    }

    // Checked before anything is created or written: a typo in `--source`
    // must not leave a project on disk that records a reference to nothing,
    // when the entire point of the flag is to hand the agent something real
    // to look at from the first turn.
    if let Some(source) = &args.source {
        std::fs::metadata(source).map_err(|error| Error::io(source, error))?;
    }

    let mut project = Project::init(&args.path)?;
    if let Some(prompt) = &args.prompt {
        project.metadata_mut().prompt = Some(prompt.clone());
    }
    if let Some(source) = &args.source {
        // `ReferenceKind::Source`: "the thing being reproduced, traced or
        // vectorised" — exactly what `--source` means. Stored verbatim, the
        // same as `set_reference` stores whatever `src` it is given.
        project
            .metadata_mut()
            .references
            .push(Reference::new(source.clone(), ReferenceKind::Source));
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
            source: None,
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

    #[test]
    fn a_source_image_ends_up_in_the_saved_project() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        let mockup = directory.path().join("mockup.png");
        std::fs::write(&mockup, b"not a real png").unwrap();

        let mut sourced = args(&path);
        sourced.source = Some(mockup.clone());
        run(&sourced, &mut Vec::new()).unwrap();

        let project = Project::open(&path).unwrap();
        let reference = &project.metadata().references[0];
        assert_eq!(reference.src, mockup);
        assert_eq!(reference.kind, ReferenceKind::Source);
    }

    #[test]
    fn a_missing_source_image_is_refused_and_nothing_is_created() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");

        let mut sourced = args(&path);
        sourced.source = Some(directory.path().join("missing.png"));

        let error = run(&sourced, &mut Vec::new()).unwrap_err();
        assert!(matches!(error, Error::Io { .. }));
        assert!(!path.exists(), "a failed init must not create the project");
    }

    #[test]
    fn a_source_image_and_a_prompt_can_both_be_given() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        let mockup = directory.path().join("mockup.png");
        std::fs::write(&mockup, b"not a real png").unwrap();

        let mut both = args(&path);
        both.prompt = Some("keep the mark, drop the wordmark".to_owned());
        both.source = Some(mockup.clone());
        run(&both, &mut Vec::new()).unwrap();

        let project = Project::open(&path).unwrap();
        assert_eq!(
            project.metadata().prompt.as_deref(),
            Some("keep the mark, drop the wordmark")
        );
        assert_eq!(project.metadata().references[0].src, mockup);
    }
}
