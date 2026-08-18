//! The project palette.
//!
//! Declarative for now: Shaipe records the palette and reports on it, but the
//! renderer never consults it. Binding artwork to palette entries needs either
//! CSS custom properties, which `resvg` does not implement, or a Shaipe-owned
//! `<style>` block; both are deliberately deferred. The data model here is the
//! one a colour picker and an import/export layer will need, so that adding
//! them later is additive rather than a rewrite.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// A straight-alpha 8-bit RGBA colour.
///
/// Straight, not premultiplied: this is the value a human typed and the value
/// Shaipe writes back. Premultiplication is `tiny-skia`'s business, at the
/// point of rasterisation, and doing it earlier loses colour in transparent
/// pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha, 255 being opaque.
    pub a: u8,
}

impl Rgba {
    /// Fully transparent.
    pub const TRANSPARENT: Self = Self::new(0, 0, 0, 0);

    /// An opaque colour from its components.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Whether this colour is fully transparent.
    #[must_use]
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }
}

impl fmt::Display for Rgba {
    /// Round-trips [`FromStr`].
    ///
    /// The alpha channel is only emitted when it is not opaque, so the common
    /// case reads as the six-digit hex a designer expects rather than as
    /// `#f05032ff`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == u8::MAX {
            write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            write!(
                f,
                "#{:02x}{:02x}{:02x}{:02x}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

impl FromStr for Rgba {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let invalid = || Error::InvalidColour {
            value: s.to_owned(),
        };
        let hex = s.strip_prefix('#').ok_or_else(invalid)?;

        // The short forms duplicate each nibble rather than scaling it, which
        // is what CSS specifies: `#f00` is `#ff0000`, not `#f00000`.
        let expand = |n: u8| n * 0x11;
        let nibble = |c: u8| {
            char::from(c)
                .to_digit(16)
                .map(|d| d as u8)
                .ok_or_else(invalid)
        };

        let bytes = hex.as_bytes();
        match bytes.len() {
            3 | 4 => {
                let mut channels = [u8::MAX; 4];
                for (channel, &byte) in channels.iter_mut().zip(bytes) {
                    *channel = expand(nibble(byte)?);
                }
                Ok(Self::new(
                    channels[0],
                    channels[1],
                    channels[2],
                    channels[3],
                ))
            }
            6 | 8 => {
                let mut channels = [u8::MAX; 4];
                for (channel, pair) in channels.iter_mut().zip(bytes.chunks_exact(2)) {
                    *channel = nibble(pair[0])? << 4 | nibble(pair[1])?;
                }
                Ok(Self::new(
                    channels[0],
                    channels[1],
                    channels[2],
                    channels[3],
                ))
            }
            _ => Err(invalid()),
        }
    }
}

/// What a colour is *for*, as opposed to what it looks like.
///
/// An open set: `Role::Other` keeps a project that names a role this build has
/// never heard of readable instead of rejected, which is the whole point of
/// versioning the schema rather than freezing it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Role {
    /// The colour the eye is meant to land on.
    Accent,
    /// The dominant brand colour.
    Primary,
    /// A supporting brand colour.
    Secondary,
    /// What the asset sits on.
    Background,
    /// Text and other content drawn over the background.
    Foreground,
    /// Anything this build does not have a name for.
    Other(String),
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Accent => "accent",
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Background => "background",
            Self::Foreground => "foreground",
            Self::Other(other) => other,
        })
    }
}

impl From<&str> for Role {
    fn from(value: &str) -> Self {
        match value {
            "accent" => Self::Accent,
            "primary" => Self::Primary,
            "secondary" => Self::Secondary,
            "background" => Self::Background,
            "foreground" => Self::Foreground,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// A named colour in a project's palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colour {
    /// How the project refers to it, for example `accent` or `background-dark`.
    pub name: String,
    /// The colour itself.
    pub value: Rgba,
    /// What it is for, when the project says.
    pub role: Option<Role>,
}

/// A project's colours, in declaration order.
///
/// Ordered, not a map: the order is the designer's, it survives a round trip
/// through the file, and a palette small enough to display in a TUI pane is
/// small enough to search linearly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Palette {
    colours: Vec<Colour>,
}

impl Palette {
    /// An empty palette.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            colours: Vec::new(),
        }
    }

    /// Append a colour.
    pub fn push(&mut self, colour: Colour) {
        self.colours.push(colour);
    }

    /// The colours, in declaration order.
    #[must_use]
    pub fn colours(&self) -> &[Colour] {
        &self.colours
    }

    /// Look a colour up by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Colour> {
        self.colours.iter().find(|colour| colour.name == name)
    }

    /// Look a colour up by name, mutably.
    ///
    /// The seam local palette editing will be built on.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Colour> {
        self.colours.iter_mut().find(|colour| colour.name == name)
    }

    /// The colour at a position, mutably.
    ///
    /// By position rather than by name, because the workspace's palette editor
    /// can change the name — and looking one up by the name being retyped
    /// would stop finding it halfway through the first keystroke.
    pub fn colour_mut(&mut self, index: usize) -> Option<&mut Colour> {
        self.colours.get_mut(index)
    }

    /// The first colour filling a given role, if any.
    #[must_use]
    pub fn by_role(&self, role: &Role) -> Option<&Colour> {
        self.colours
            .iter()
            .find(|colour| colour.role.as_ref() == Some(role))
    }

    /// How many colours the palette holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.colours.len()
    }

    /// Whether the palette holds no colours.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.colours.is_empty()
    }
}

impl FromIterator<Colour> for Palette {
    fn from_iter<T: IntoIterator<Item = Colour>>(iter: T) -> Self {
        Self {
            colours: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("#f05032", Rgba::new(0xf0, 0x50, 0x32, 0xff))]
    #[case("#F05032", Rgba::new(0xf0, 0x50, 0x32, 0xff))]
    #[case("#f0503280", Rgba::new(0xf0, 0x50, 0x32, 0x80))]
    #[case("#fff", Rgba::new(0xff, 0xff, 0xff, 0xff))]
    #[case("#0000", Rgba::new(0x00, 0x00, 0x00, 0x00))]
    fn every_css_hex_form_parses(#[case] input: &str, #[case] expected: Rgba) {
        assert_eq!(input.parse::<Rgba>().unwrap(), expected);
    }

    #[rstest]
    #[case("f05032")] // no `#`
    #[case("#f0503")] // five digits is not a form
    #[case("#gggggg")] // not hex
    #[case("")]
    fn anything_that_is_not_a_css_hex_colour_is_rejected(#[case] input: &str) {
        assert!(input.parse::<Rgba>().is_err());
    }

    #[rstest]
    #[case("#f05032")]
    #[case("#f0503280")]
    fn displaying_a_colour_round_trips_through_parsing(#[case] input: &str) {
        let colour: Rgba = input.parse().unwrap();
        assert_eq!(colour.to_string(), input);
    }

    #[test]
    fn a_short_hex_colour_expands_by_duplicating_nibbles_not_by_padding() {
        // `#f00` is `#ff0000`. Padding with zeroes would give `#f00000`, a
        // visibly different red, and this is the classic way to get it wrong.
        assert_eq!("#f00".parse::<Rgba>().unwrap(), Rgba::new(0xff, 0, 0, 0xff));
    }

    #[test]
    fn an_unknown_role_is_preserved_rather_than_rejected() {
        // A project written by a newer Shaipe must stay readable by an older
        // one, or the schema version is doing no work.
        assert_eq!(Role::from("hyperlink"), Role::Other("hyperlink".to_owned()));
        assert_eq!(Role::from("hyperlink").to_string(), "hyperlink");
    }

    #[test]
    fn a_palette_finds_colours_by_name_and_by_role() {
        let palette: Palette = [
            Colour {
                name: "accent".to_owned(),
                value: Rgba::new(0xf0, 0x50, 0x32, 0xff),
                role: Some(Role::Accent),
            },
            Colour {
                name: "ink".to_owned(),
                value: Rgba::new(0x18, 0x18, 0x1b, 0xff),
                role: None,
            },
        ]
        .into_iter()
        .collect();

        assert_eq!(palette.get("ink").unwrap().value.to_string(), "#18181b");
        assert_eq!(palette.by_role(&Role::Accent).unwrap().name, "accent");
        assert!(palette.by_role(&Role::Background).is_none());
    }
}
