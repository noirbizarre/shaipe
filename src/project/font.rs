//! Fonts a project depends on.
//!
//! `resvg` has no `@font-face` support whatsoever — the string does not appear
//! anywhere in its source — so an SVG cannot carry its own fonts. The only
//! input is the `fontdb::Database` the caller populates, which makes font
//! resolution Shaipe's problem rather than the document's.
//!
//! Declaring fonts here, with paths resolved relative to the project file, is
//! what makes a render reproducible: commit the font next to the project and
//! CI draws the same glyphs your machine did. Undeclared families fall back to
//! system fonts with a warning, which is convenient locally and a determinism
//! hazard everywhere else — hence `--strict-fonts`.

use std::path::PathBuf;

/// A font file the project depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    /// The family name the SVG's `font-family` refers to.
    pub family: String,
    /// Where the font file lives, relative to the project file.
    pub src: PathBuf,
}

impl Font {
    /// Declare a font family backed by a file.
    #[must_use]
    pub fn new(family: impl Into<String>, src: impl Into<PathBuf>) -> Self {
        Self {
            family: family.into(),
            src: src.into(),
        }
    }
}
