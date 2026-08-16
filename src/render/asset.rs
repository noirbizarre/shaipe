//! What a render produces.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::project::spec::{Format, RenderSpec};

/// An asset, in memory.
///
/// Produced without touching the filesystem, so that the TUI can preview a
/// render it never writes and the tool registry can hand one to an agent.
/// Writing is a separate, explicit step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedAsset {
    /// The specification that produced it.
    pub spec: RenderSpec,
    /// The encoded bytes: a PNG, or SVG text.
    pub bytes: Vec<u8>,
}

impl RenderedAsset {
    /// The file name this asset should be written as.
    #[must_use]
    pub fn file_name(&self) -> String {
        self.spec.file_name()
    }

    /// How it is encoded.
    #[must_use]
    pub fn format(&self) -> Format {
        self.spec.format
    }

    /// Write the asset into a directory, creating it if needed.
    ///
    /// Returns the path written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`], naming the directory or the file that failed.
    pub fn write_to(&self, directory: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(directory).map_err(|source| Error::io(directory, source))?;
        let path = directory.join(self.file_name());
        std::fs::write(&path, &self.bytes).map_err(|source| Error::io(&path, source))?;
        Ok(path)
    }
}
