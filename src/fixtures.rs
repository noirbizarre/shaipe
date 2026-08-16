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
pub const PROJECT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 64 64" width="64" height="64">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:prompt>A square and a bar.</shaipe:prompt>
      <shaipe:palette>
        <shaipe:color name="accent" value="#f05032" role="accent"/>
        <shaipe:color name="ink" value="#18181b"/>
      </shaipe:palette>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
        <shaipe:variant name="wordmark" ref="mark-wide"/>
      </shaipe:variants>
      <shaipe:renders>
        <shaipe:render name="favicon-32" variant="icon" width="32"/>
        <shaipe:render name="banner" variant="wordmark" width="128" height="32" background="#18181b"/>
      </shaipe:renders>
    </shaipe:project>
  </metadata>
  <symbol id="icon" viewBox="0 0 64 64"><rect width="64" height="64" fill="#f05032"/></symbol>
  <symbol id="mark-wide" viewBox="0 0 256 64"><rect width="256" height="64" fill="#f05032"/></symbol>
  <use href="#icon" width="64" height="64"/>
</svg>
"##;

/// [`PROJECT`], opened.
///
/// # Panics
///
/// If the fixture stops being a valid project, which is the point.
#[must_use]
pub fn project() -> Project {
    Project::from_source("logo.svg", PROJECT.to_owned()).expect("the fixture is a valid project")
}
