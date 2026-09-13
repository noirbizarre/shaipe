//! Fonts a project depends on.
//!
//! `resvg` has no `@font-face` support whatsoever — the string does not appear
//! anywhere in its source — so an SVG cannot carry its own fonts. The only
//! input is the `fontdb::Database` the caller populates, which makes font
//! resolution Shaipe's problem rather than the document's.
//!
//! Declaring fonts here is what makes a render reproducible, one of two ways:
//! a local file, committed next to the project (`src`, paths resolved
//! relative to the project file the same as a reference or a render's
//! output); or a checksum-pinned URL (`href`/`sha256`), fetched once and
//! cached content-addressed by that hash — see ADR 015 and
//! [`crate::fonts`], which is where either kind is actually turned into
//! bytes. Undeclared families fall back to system fonts with a warning,
//! which is convenient locally and a determinism hazard everywhere else —
//! hence `--strict-fonts`.

use std::path::PathBuf;

/// Where a declared font's bytes actually come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSource {
    /// A file committed alongside the project, resolved the same way a
    /// reference or a render's declared output directory is.
    Local(PathBuf),
    /// A checksum-pinned URL. `sha256` is not optional here — it is what
    /// makes the cache key meaningful and what makes a warm-cache render
    /// reproducible without ever touching the network again; a URL with
    /// nothing pinning its content would just be system-font fallback with
    /// extra steps.
    Remote {
        /// Where to fetch it from, the first time.
        href: String,
        /// The downloaded bytes' expected SHA-256, as 64 lowercase hex
        /// characters. Checked before the bytes are cached, and again every
        /// time a cache hit is read back, so a corrupted cache is caught
        /// the same way a tampered-with download is.
        sha256: String,
    },
}

/// A font family the project depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    /// The family name the SVG's `font-family` refers to.
    pub family: String,
    /// Where its bytes come from.
    pub source: FontSource,
}

impl Font {
    /// Declare a font family backed by a committed file.
    #[must_use]
    pub fn new(family: impl Into<String>, src: impl Into<PathBuf>) -> Self {
        Self {
            family: family.into(),
            source: FontSource::Local(src.into()),
        }
    }

    /// Declare a font family backed by a checksum-pinned URL.
    #[must_use]
    pub fn remote(
        family: impl Into<String>,
        href: impl Into<String>,
        sha256: impl Into<String>,
    ) -> Self {
        Self {
            family: family.into(),
            source: FontSource::Remote {
                href: href.into(),
                sha256: sha256.into(),
            },
        }
    }
}
