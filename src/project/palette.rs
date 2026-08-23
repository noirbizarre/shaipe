//! The project palette.
//!
//! Artwork binds to a palette entry with a `shaipe:fill`/`shaipe:stroke`
//! attribute naming a [`Colour`] by its `name`, read alongside the element's
//! ordinary `fill`/`stroke` rather than instead of it — `resvg` does not
//! implement CSS custom properties, and this way the document stays a correct,
//! ordinary SVG whether or not anything ever resolves the binding. Editing a
//! colour's value (`Project::restyle`, called by `set_palette_colour` and by
//! the workspace's own palette pane) finds every element bound to that name
//! and splices its literal attribute to match, immediately — there is no
//! render-time substitution step, and nothing in [`crate::render`] needs to
//! know the palette exists.

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

/// A colour in hue/saturation/lightness form, for editing.
///
/// `Rgba` is the value every part of a project stores, parses and writes back
/// — hex is what a designer *names* a colour by. Nobody adjusts one by typing
/// hex digits, though: hue, saturation and lightness are the three knobs an
/// eye actually turns, so the workspace's colour picker keeps its working
/// state here and converts to and from `Rgba` only at the edges, where a
/// value needs storing or a stored value needs sliders. No alpha: it is not
/// part of hue, saturation or lightness, and a caller that needs it keeps it
/// alongside, the way [`Rgba`] keeps `a` alongside `r`/`g`/`b`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsl {
    /// Degrees around the colour wheel, `0.0..360.0`.
    pub h: f32,
    /// `0.0..=1.0`, grey to fully saturated.
    pub s: f32,
    /// `0.0..=1.0`, black to white.
    pub l: f32,
}

impl From<Rgba> for Hsl {
    /// The standard conversion, ignoring alpha.
    fn from(colour: Rgba) -> Self {
        let r = f32::from(colour.r) / 255.0;
        let g = f32::from(colour.g) / 255.0;
        let b = f32::from(colour.b) / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let l = f32::midpoint(max, min);

        // Grey has no hue and no saturation: `delta` is zero and every branch
        // below would divide by it.
        if delta == 0.0 {
            return Self { h: 0.0, s: 0.0, l };
        }

        let s = delta / (1.0 - (2.0 * l - 1.0).abs());
        let h = 60.0
            * if max == r {
                ((g - b) / delta).rem_euclid(6.0)
            } else if max == g {
                (b - r) / delta + 2.0
            } else {
                (r - g) / delta + 4.0
            };

        Self { h, s, l }
    }
}

impl Hsl {
    /// The 8-bit RGB components this colour rounds to.
    ///
    /// Alpha is not this type's business — see the struct's own doc comment
    /// — so a caller combines this with whatever alpha it is keeping
    /// alongside, typically via [`Rgba::new`].
    #[must_use]
    pub fn to_rgb(self) -> (u8, u8, u8) {
        // The standard HSL-to-RGB construction: `c` is the chroma, `x` the
        // second-largest component, and `m` the offset that lands the
        // largest component's floor at zero rather than at `m`.
        let c = (1.0 - (2.0 * self.l - 1.0).abs()) * self.s;
        let h_prime = self.h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
        let m = self.l - c / 2.0;

        let (r1, g1, b1) = match h_prime as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };

        let channel = |value: f32| (((value + m) * 255.0).round().clamp(0.0, 255.0)) as u8;
        (channel(r1), channel(g1), channel(b1))
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

    #[rstest]
    // Pure red, green and blue: exact, because none of these divisions land
    // on a value a `f32` cannot represent precisely.
    #[case(Rgba::new(0xff, 0x00, 0x00, 0xff), Hsl { h: 0.0, s: 1.0, l: 0.5 })]
    #[case(Rgba::new(0x00, 0xff, 0x00, 0xff), Hsl { h: 120.0, s: 1.0, l: 0.5 })]
    #[case(Rgba::new(0x00, 0x00, 0xff, 0xff), Hsl { h: 240.0, s: 1.0, l: 0.5 })]
    // Black and white: no hue, no saturation, and lightness at either end.
    #[case(Rgba::new(0x00, 0x00, 0x00, 0xff), Hsl { h: 0.0, s: 0.0, l: 0.0 })]
    #[case(Rgba::new(0xff, 0xff, 0xff, 0xff), Hsl { h: 0.0, s: 0.0, l: 1.0 })]
    fn converting_a_known_colour_to_hsl_matches_the_textbook_values(
        #[case] rgba: Rgba,
        #[case] expected: Hsl,
    ) {
        let hsl = Hsl::from(rgba);
        assert!((hsl.h - expected.h).abs() < 0.01, "{hsl:?}");
        assert!((hsl.s - expected.s).abs() < 0.01, "{hsl:?}");
        assert!((hsl.l - expected.l).abs() < 0.01, "{hsl:?}");
    }

    #[rstest]
    #[case(Rgba::new(0xf0, 0x50, 0x32, 0xff))] // the fixture's own accent
    #[case(Rgba::new(0x18, 0x18, 0x1b, 0xff))]
    #[case(Rgba::new(0x00, 0x00, 0x00, 0xff))]
    #[case(Rgba::new(0xff, 0xff, 0xff, 0xff))]
    #[case(Rgba::new(0x7f, 0x3c, 0xa1, 0xff))]
    fn a_colour_survives_a_round_trip_through_hsl(#[case] rgba: Rgba) {
        // Off by a shade is float rounding, not a bug: an eight-bit channel
        // does not always land on a value `f32` arithmetic can hit exactly
        // going the other way. A whole point of difference would be a
        // visible shift and is what this guards against.
        let (r, g, b) = Hsl::from(rgba).to_rgb();
        assert!(r.abs_diff(rgba.r) <= 1, "red drifted: {rgba:?} -> {r}");
        assert!(g.abs_diff(rgba.g) <= 1, "green drifted: {rgba:?} -> {g}");
        assert!(b.abs_diff(rgba.b) <= 1, "blue drifted: {rgba:?} -> {b}");
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
