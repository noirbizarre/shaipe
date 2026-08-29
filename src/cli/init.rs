//! `shaipe init`.
//!
//! Creates a project from nothing: a minimal placeholder icon, declared as
//! the primary variant, ready to open in the workspace or hand to an agent.

use std::io::Write;
use std::path::{Path, PathBuf};

use shaipe::project::{Reference, ReferenceKind};
use shaipe::{Error, Project, Result};

use crate::cli::InitArgs;

/// `path`, as it should be recorded in a reference: relative to `base` rather
/// than to wherever the command happened to run.
///
/// A reference is resolved against the project's own directory (see
/// [`shaipe::project::Project::resolve`]), not the shell's working
/// directory — so `shaipe init subdir/logo.svg --source mockup.png`, with
/// `mockup.png` sitting beside the shell rather than beside `subdir/`, must
/// not store `mockup.png` verbatim: an agent resolving it later would look
/// for `subdir/mockup.png`, which is not the file that was given.
///
/// Neither path has to exist for this to make sense — [`std::path::absolute`]
/// only joins onto the current directory and normalises `.`/`..`
/// lexically, it never touches the filesystem — which matters for `base`:
/// the project file itself is not written yet when this runs.
fn relative_to(base: &Path, target: &Path) -> Result<PathBuf> {
    let absolute = |path: &Path| std::path::absolute(path).map_err(|error| Error::io(path, error));
    let base = absolute(base)?;
    let target = absolute(target)?;

    let mut base_components = base.components().peekable();
    let mut target_components = target.components().peekable();

    // The shared prefix — typically the whole of `base` — is neither `..`
    // nor repeated, so what is left of each side is exactly the detour
    // between them.
    while base_components.peek().is_some() && base_components.peek() == target_components.peek() {
        base_components.next();
        target_components.next();
    }

    let mut relative = PathBuf::new();
    for _ in base_components {
        relative.push("..");
    }
    relative.extend(target_components);

    Ok(relative)
}

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

    // Checked before anything is created or written: a typo in `--source` or
    // `--inspiration` must not leave a project on disk that records a
    // reference to nothing, when the entire point of the flags is to hand the
    // agent something real to look at from the first turn.
    if let Some(source) = &args.source {
        std::fs::metadata(source).map_err(|error| Error::io(source, error))?;
    }
    for inspiration in &args.inspiration {
        std::fs::metadata(inspiration).map_err(|error| Error::io(inspiration, error))?;
    }

    let mut project = Project::init(&args.path)?;
    let base = project.base_directory();
    if let Some(prompt) = &args.prompt {
        project.metadata_mut().prompt = Some(prompt.clone());
    }
    if let Some(source) = &args.source {
        // `ReferenceKind::Source`: "the thing being reproduced, traced or
        // vectorised" — exactly what `--source` means. Re-expressed relative
        // to the project rather than stored verbatim, so it still resolves
        // once the shell's own working directory is gone.
        let src = relative_to(&base, source)?;
        project
            .metadata_mut()
            .references
            .push(Reference::new(src, ReferenceKind::Source));
    }
    for inspiration in &args.inspiration {
        // `ReferenceKind::Inspiration`: "cues, not to copy" — a mood board
        // rather than something to trace. Each `--inspiration` becomes its
        // own reference, in the order given.
        let src = relative_to(&base, inspiration)?;
        project
            .metadata_mut()
            .references
            .push(Reference::new(src, ReferenceKind::Inspiration));
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
            inspiration: Vec::new(),
        }
    }

    #[test]
    fn a_file_beside_the_base_is_relative_to_it_by_name_alone() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path();
        let target = base.join("mockup.png");

        assert_eq!(
            relative_to(base, &target).unwrap(),
            std::path::Path::new("mockup.png")
        );
    }

    #[test]
    fn a_file_outside_the_base_gets_one_detour_per_directory_between_them() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("a").join("b");
        let target = directory.path().join("mockup.png");

        assert_eq!(
            relative_to(&base, &target).unwrap(),
            std::path::Path::new("../../mockup.png")
        );
    }

    #[test]
    fn a_file_in_a_sibling_directory_climbs_out_and_back_in() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("a");
        let target = directory.path().join("b").join("mockup.png");

        assert_eq!(
            relative_to(&base, &target).unwrap(),
            std::path::Path::new("../b/mockup.png")
        );
    }

    #[test]
    fn a_relative_base_and_target_are_both_resolved_against_the_current_directory() {
        // Neither path has to be absolute, or to exist, for the detour
        // between them to make sense — only `std::env::current_dir` has to
        // succeed, which it does in a test process.
        assert_eq!(
            relative_to(
                std::path::Path::new("."),
                std::path::Path::new("mockup.png")
            )
            .unwrap(),
            std::path::Path::new("mockup.png")
        );
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
        // Relative to the project, not the absolute path it was given as —
        // both live in the same directory here, so that is just the name.
        assert_eq!(reference.src, std::path::Path::new("mockup.png"));
        assert_eq!(reference.kind, ReferenceKind::Source);
        assert_eq!(project.resolve(&reference.src), mockup);
    }

    #[test]
    fn a_source_image_outside_the_projects_directory_is_recorded_with_a_detour() {
        // The bug this guards: a project created in a subdirectory, sourced
        // from a file that lives beside the shell rather than beside it.
        // Storing the given path verbatim would have an agent resolving it
        // against the wrong directory the moment the shell is gone.
        let directory = tempfile::tempdir().unwrap();
        let subdirectory = directory.path().join("project");
        std::fs::create_dir(&subdirectory).unwrap();
        let path = subdirectory.join("logo.svg");
        let mockup = directory.path().join("mockup.png");
        std::fs::write(&mockup, b"not a real png").unwrap();

        let mut sourced = args(&path);
        sourced.source = Some(mockup.clone());
        run(&sourced, &mut Vec::new()).unwrap();

        let project = Project::open(&path).unwrap();
        let reference = &project.metadata().references[0];
        assert_eq!(reference.src, std::path::Path::new("../mockup.png"));
        // `resolve` joins the literal `..` rather than collapsing it, so the
        // two are only the same file once the filesystem is asked, not as
        // equal `PathBuf`s.
        assert_eq!(
            std::fs::canonicalize(project.resolve(&reference.src)).unwrap(),
            std::fs::canonicalize(&mockup).unwrap()
        );
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
        assert_eq!(
            project.metadata().references[0].src,
            std::path::Path::new("mockup.png")
        );
    }

    #[test]
    fn inspiration_images_end_up_in_the_saved_project() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        let mood_a = directory.path().join("mood-a.png");
        let mood_b = directory.path().join("mood-b.png");
        std::fs::write(&mood_a, b"not a real png").unwrap();
        std::fs::write(&mood_b, b"not a real png").unwrap();

        let mut inspired = args(&path);
        inspired.inspiration = vec![mood_a.clone(), mood_b.clone()];
        run(&inspired, &mut Vec::new()).unwrap();

        let project = Project::open(&path).unwrap();
        let references = &project.metadata().references;
        assert_eq!(references.len(), 2);
        assert_eq!(references[0].src, std::path::Path::new("mood-a.png"));
        assert_eq!(references[0].kind, ReferenceKind::Inspiration);
        assert_eq!(references[1].src, std::path::Path::new("mood-b.png"));
        assert_eq!(references[1].kind, ReferenceKind::Inspiration);
    }

    #[test]
    fn a_missing_inspiration_image_is_refused_and_nothing_is_created() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        let mood_a = directory.path().join("mood-a.png");
        std::fs::write(&mood_a, b"not a real png").unwrap();

        let mut inspired = args(&path);
        inspired.inspiration = vec![mood_a, directory.path().join("missing.png")];

        let error = run(&inspired, &mut Vec::new()).unwrap_err();
        assert!(matches!(error, Error::Io { .. }));
        assert!(!path.exists(), "a failed init must not create the project");
    }

    #[test]
    fn source_and_inspiration_can_both_be_given() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        let mockup = directory.path().join("mockup.png");
        let mood = directory.path().join("mood.png");
        std::fs::write(&mockup, b"not a real png").unwrap();
        std::fs::write(&mood, b"not a real png").unwrap();

        let mut both = args(&path);
        both.source = Some(mockup.clone());
        both.inspiration = vec![mood.clone()];
        run(&both, &mut Vec::new()).unwrap();

        let project = Project::open(&path).unwrap();
        let references = &project.metadata().references;
        assert_eq!(references[0].src, std::path::Path::new("mockup.png"));
        assert_eq!(references[0].kind, ReferenceKind::Source);
        assert_eq!(references[1].src, std::path::Path::new("mood.png"));
        assert_eq!(references[1].kind, ReferenceKind::Inspiration);
    }
}
