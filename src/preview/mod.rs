//! Showing a rendered asset in a terminal.
//!
//! Terminal graphics are a set of mutually incompatible escape-sequence
//! protocols — Kitty, Sixel, iTerm2 — with uneven support and a half-block
//! fallback that works everywhere. That mess is `ratatui-image`'s to carry,
//! and this module is the wall that keeps it there. See ADR-006.
//!
//! Note the direction of the dependency. Nothing here knows what an SVG is, a
//! variant is, or a render specification is: the layer is handed an [`Image`]
//! and an area. That boundary is enforced by the `preview-isolation` hook,
//! and it is what stops a terminal graphics protocol from acquiring an
//! opinion about vector artwork.

mod image;

pub use image::Image;

use std::str::FromStr;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui_image::StatefulImage;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;

/// A terminal graphics protocol.
///
/// Mirrors `ratatui_image::picker::ProtocolType` rather than re-exporting it,
/// so that the choice of that crate stays an implementation detail and
/// `--preview` keeps a stable spelling regardless of what it renames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// Ask the terminal what it supports.
    #[default]
    Auto,
    /// The Kitty graphics protocol.
    Kitty,
    /// Sixel.
    Sixel,
    /// The iTerm2 inline image protocol.
    Iterm2,
    /// Unicode half-blocks: everywhere, at half the vertical resolution.
    Blocks,
}

impl Backend {
    /// The protocol this names, or `None` for [`Backend::Auto`].
    const fn protocol(self) -> Option<ProtocolType> {
        match self {
            Self::Auto => None,
            Self::Kitty => Some(ProtocolType::Kitty),
            Self::Sixel => Some(ProtocolType::Sixel),
            Self::Iterm2 => Some(ProtocolType::Iterm2),
            Self::Blocks => Some(ProtocolType::Halfblocks),
        }
    }
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "kitty" => Ok(Self::Kitty),
            "sixel" => Ok(Self::Sixel),
            "iterm2" => Ok(Self::Iterm2),
            "blocks" | "halfblocks" => Ok(Self::Blocks),
            other => Err(format!(
                "unknown preview backend `{other}`, expected `auto`, `kitty`, \
                 `sixel`, `iterm2` or `blocks`"
            )),
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Kitty => "kitty",
            Self::Sixel => "sixel",
            Self::Iterm2 => "iterm2",
            Self::Blocks => "blocks",
        })
    }
}

/// What transparency is flattened against in the half-block fallback.
///
/// Mid grey, so that neither near-black nor near-white artwork disappears
/// into it. Deliberately not an attempt to match the terminal's own
/// background: that is not reliably knowable, and guessing it wrong is how a
/// preview ends up showing nothing at all.
const FALLBACK_BACKGROUND: ::image::Rgba<u8> = ::image::Rgba([128, 128, 128, 255]);

/// The name to show for a protocol.
const fn name_of(protocol: ProtocolType) -> &'static str {
    match protocol {
        ProtocolType::Halfblocks => "blocks",
        ProtocolType::Sixel => "sixel",
        ProtocolType::Kitty => "kitty",
        ProtocolType::Iterm2 => "iterm2",
    }
}

/// A terminal that can be shown an image.
///
/// Owns the encoded form of whatever is currently on screen. Re-encoding on
/// every frame would re-transmit the whole image to the terminal four times a
/// second, so the encoded protocol state is kept and reused until either the
/// image or the area it is drawn in changes — which `ratatui-image` detects
/// for itself, given the same state back each frame.
pub struct Preview {
    picker: Picker,
    protocol: Option<StatefulProtocol>,
    /// Which image the protocol holds. `None` means nothing is prepared yet.
    generation: Option<u64>,
}

impl Preview {
    /// Choose a backend and prepare to draw with it.
    ///
    /// `Backend::Auto` asks the terminal directly, which is more accurate than
    /// guessing from environment variables and cannot hang: the query is
    /// bounded by a timeout, and falls back to half-blocks when nothing
    /// answers.
    ///
    /// # Panics
    ///
    /// Never. A failed query degrades to half-blocks, because a workspace that
    /// refuses to open because it could not identify the terminal would be
    /// worse than one with a coarse preview.
    #[must_use]
    pub fn detect(requested: Backend) -> Self {
        // Half-blocks need no capabilities, so asking the terminal about them
        // is a pointless round trip on a path chosen precisely because the
        // fancy ones did not work.
        let mut picker = if requested == Backend::Blocks {
            Picker::halfblocks()
        } else {
            Picker::from_query_stdio().unwrap_or_else(|error| {
                log::debug!("terminal graphics query failed ({error}); using half-blocks");
                Picker::halfblocks()
            })
        };

        // An explicit request overrides detection. Detection is still run
        // first, because the font size it reports is what makes an image come
        // out the right shape, and only the protocol is being overridden.
        if let Some(protocol) = requested.protocol() {
            picker.set_protocol_type(protocol);
        }

        // A terminal cell has no alpha, so half-blocks must flatten
        // transparency against *something*, and that something defaults to
        // black — which makes a dark logo on a transparent background
        // invisible. A neutral grey is wrong for no artwork in particular
        // instead of catastrophically wrong for dark artwork.
        //
        // Only for half-blocks: the real graphics protocols carry an alpha
        // channel, and setting this would flatten an image they could have
        // drawn correctly.
        if picker.protocol_type() == ProtocolType::Halfblocks {
            picker.set_background_color(Some(FALLBACK_BACKGROUND));
        }

        Self {
            picker,
            protocol: None,
            generation: None,
        }
    }

    /// The name of the protocol in use.
    #[must_use]
    pub fn name(&self) -> &'static str {
        name_of(self.picker.protocol_type())
    }

    /// Forget the prepared image, so the next draw re-encodes.
    pub fn invalidate(&mut self) {
        self.protocol = None;
        self.generation = None;
    }

    /// Draw an image into an area.
    ///
    /// `generation` identifies the image: when it changes, the image is
    /// re-encoded. Passing the same value with different pixels would show the
    /// stale image, which is why the caller increments it on every change
    /// rather than comparing pixels.
    pub fn draw(&mut self, image: &Image, generation: u64, area: Rect, frame: &mut Frame<'_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        if self.generation != Some(generation) {
            self.protocol = Some(self.picker.new_resize_protocol(image.to_dynamic()));
            self.generation = Some(generation);
        }

        let Some(protocol) = self.protocol.as_mut() else {
            return;
        };

        // `StatefulImage` resizes and re-encodes only when the area changed,
        // which is what makes redrawing at the tick rate cheap.
        frame.render_stateful_widget(StatefulImage::default(), area, protocol);

        // Encoding happens inside the widget, so a failure is only observable
        // afterwards. Reported rather than swallowed, because the symptom
        // would otherwise be an empty pane with no explanation anywhere.
        if let Some(Err(error)) = protocol.last_encoding_result() {
            log::debug!("preview encoding failed: {error}");
        }
    }
}

impl std::fmt::Debug for Preview {
    /// Hand-written because `Picker` and `StatefulProtocol` are not `Debug`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview")
            .field("backend", &self.name())
            .field("prepared", &self.generation)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn every_backend_name_round_trips_through_parsing() {
        for backend in [
            Backend::Auto,
            Backend::Kitty,
            Backend::Sixel,
            Backend::Iterm2,
            Backend::Blocks,
        ] {
            assert_eq!(backend.to_string().parse::<Backend>().unwrap(), backend);
        }
    }

    #[test]
    fn an_unknown_backend_name_lists_the_ones_that_exist() {
        let error = "ascii".parse::<Backend>().unwrap_err();
        for expected in ["kitty", "sixel", "iterm2", "blocks"] {
            assert!(error.contains(expected), "{expected} missing from: {error}");
        }
    }

    #[test]
    fn halfblocks_is_spelled_both_ways() {
        // `ratatui-image` calls it `halfblocks` and this crate calls it
        // `blocks`; a user who reaches for either should not get an error.
        assert_eq!("halfblocks".parse::<Backend>().unwrap(), Backend::Blocks);
        assert_eq!("blocks".parse::<Backend>().unwrap(), Backend::Blocks);
    }

    #[test]
    fn an_explicitly_requested_backend_is_used_without_detection() {
        // `--preview blocks` has to work on a Kitty terminal, or it is useless
        // as an escape hatch when detection gets it right but rendering wrong.
        assert_eq!(Preview::detect(Backend::Blocks).name(), "blocks");
    }

    #[test]
    fn detection_falls_back_to_halfblocks_when_no_terminal_answers() {
        // The test runner is not a terminal. Opening the workspace must not
        // depend on the query succeeding.
        assert_eq!(Preview::detect(Backend::Auto).name(), "blocks");
    }
}
