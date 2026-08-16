//! Showing a rendered asset in a terminal.
//!
//! Terminal graphics are a mess of mutually incompatible, unevenly supported
//! escape sequences, and that mess must not leak into the renderer or the
//! project model. So it is confined here, behind [`Preview`]: a backend is
//! handed pixels and an area, and how it gets them onto the screen is its own
//! business.
//!
//! Two backends exist. [`kitty`] uses the Kitty graphics protocol and draws
//! actual pixels; [`blocks`] uses half-block characters and works in every
//! terminal there is, including one being scraped by CI. Sixel and iTerm2
//! would be new modules and nothing else.
//!
//! Note the direction of the dependency: preview knows about pixels, and
//! nothing about projects, variants or rendering. It is handed a
//! [`crate::render::RenderedAsset`]'s worth of pixels and asked to draw them.

pub mod blocks;
pub mod kitty;

mod image;

pub use image::Image;

use std::str::FromStr;

use ratatui::layout::Rect;

/// A terminal preview backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// Choose based on what the terminal appears to be.
    #[default]
    Auto,
    /// The Kitty graphics protocol: real pixels.
    Kitty,
    /// Unicode half-blocks: everywhere, at half the vertical resolution.
    Blocks,
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "kitty" => Ok(Self::Kitty),
            "blocks" => Ok(Self::Blocks),
            other => Err(format!(
                "unknown preview backend `{other}`, expected `auto`, `kitty` or `blocks`"
            )),
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Kitty => "kitty",
            Self::Blocks => "blocks",
        })
    }
}

/// Somewhere an [`Image`] can be drawn.
///
/// Implementations draw *outside* ratatui's buffer — they write escape
/// sequences straight to the terminal — which is why this is a trait of its
/// own rather than a `Widget`. Ratatui is told what the backend covered so it
/// does not paint over it.
pub trait Preview {
    /// A name to show the user, so a preview that looks wrong is diagnosable.
    fn name(&self) -> &'static str;

    /// Draw an image into an area of the terminal.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if the terminal cannot be written to.
    /// A preview failing must never take the application down with it.
    fn draw(&mut self, image: &Image, area: Rect) -> std::io::Result<()>;

    /// Forget anything previously drawn.
    ///
    /// Called before a redraw. Backends that place images out of band have to
    /// remove the old one themselves; the half-block backend has nothing to do
    /// because ratatui already cleared its cells.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if the terminal cannot be written to.
    fn clear(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Choose a backend.
///
/// [`Backend::Auto`] asks the environment rather than querying the terminal:
/// a query means writing an escape sequence and waiting for a reply, which
/// hangs on a terminal that does not answer and is unpleasant to get right
/// when something else is already reading the same input. The environment is
/// less precise and cannot hang, and `--preview` overrides it when the guess
/// is wrong.
#[must_use]
pub fn detect(requested: Backend) -> Box<dyn Preview> {
    match requested {
        Backend::Kitty => Box::new(kitty::Kitty::new()),
        Backend::Blocks => Box::new(blocks::Blocks),
        Backend::Auto => {
            if kitty::is_supported() {
                Box::new(kitty::Kitty::new())
            } else {
                Box::new(blocks::Blocks)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_backend_name_round_trips_through_parsing() {
        for backend in [Backend::Auto, Backend::Kitty, Backend::Blocks] {
            assert_eq!(backend.to_string().parse::<Backend>().unwrap(), backend);
        }
    }

    #[test]
    fn an_unknown_backend_name_lists_the_ones_that_exist() {
        let error = "sixel".parse::<Backend>().unwrap_err();
        assert!(error.contains("kitty"), "{error}");
        assert!(error.contains("blocks"), "{error}");
    }

    #[test]
    fn an_explicitly_requested_backend_is_used_without_detection() {
        // `--preview blocks` has to work on a Kitty terminal, or it is useless
        // as an escape hatch when detection gets it right but rendering wrong.
        assert_eq!(detect(Backend::Blocks).name(), "blocks");
        assert_eq!(detect(Backend::Kitty).name(), "kitty");
    }
}
