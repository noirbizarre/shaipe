//! Test fixtures shared across the crate's unit tests.
//!
//! Compiled only under `cfg(test)`. One fixture, deliberately: a project that
//! exercises every part of the format at once is worth more than a scattering
//! of one-off strings, because a change that breaks the format breaks it here
//! visibly rather than in whichever test happened to cover that corner.

use crate::project::Project;

/// A project with two variants, a palette, and a root that draws the primary
/// one so the file stays viewable.
///
/// The `icon` symbol is a solid accent-coloured square filling its own square
/// `viewBox`; `mark-wide` is the same colour at 4:1. Solid fills and simple
/// ratios are what let the renderer's tests assert on individual pixels.
///
/// It lives on disk rather than in this string so that the tests which need a
/// real path — the CLI's, which open a file — and the tests which need bytes
/// exercise the very same project.
pub const PROJECT: &str = include_str!("../tests/fixtures/logo.svg");

/// [`PROJECT`], opened.
///
/// # Panics
///
/// If the fixture stops being a valid project, which is the point.
#[must_use]
pub fn project() -> Project {
    Project::from_source("logo.svg", PROJECT.to_owned()).expect("the fixture is a valid project")
}
