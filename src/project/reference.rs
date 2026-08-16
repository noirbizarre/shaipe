//! Images and documents a project was informed by.
//!
//! References are recorded, resolved and shown; they are never rendered and
//! never read by the renderer. They exist so that the agent-facing tools have
//! something concrete to inspect, and so the raster-to-vector workflow has a
//! place to put its input when it arrives. Recording *why* a file is attached
//! is the point: "this is what we are reproducing" and "this is a mood board"
//! are very different instructions to give a vision model.

use std::path::PathBuf;

/// Why a file is attached to a project.
///
/// An open set, for the same reason [`crate::project::palette::Role`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceKind {
    /// The thing being reproduced, traced or vectorised.
    Source,
    /// Something to take cues from, not to copy.
    Inspiration,
    /// A rendering of this project, kept for comparison.
    Baseline,
    /// Anything this build does not have a name for.
    Other(String),
}

impl std::fmt::Display for ReferenceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Source => "source",
            Self::Inspiration => "inspiration",
            Self::Baseline => "baseline",
            Self::Other(other) => other,
        })
    }
}

impl From<&str> for ReferenceKind {
    fn from(value: &str) -> Self {
        match value {
            "source" => Self::Source,
            "inspiration" => Self::Inspiration,
            "baseline" => Self::Baseline,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// A file attached to the project for context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Where it lives, relative to the project file.
    pub src: PathBuf,
    /// Why it is attached.
    pub kind: ReferenceKind,
    /// What it is, in the project author's words.
    pub note: Option<String>,
}

impl Reference {
    /// Attach a file.
    #[must_use]
    pub fn new(src: impl Into<PathBuf>, kind: ReferenceKind) -> Self {
        Self {
            src: src.into(),
            kind,
            note: None,
        }
    }
}
