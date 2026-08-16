//! Declarative descriptions of assets to produce.
//!
//! A render specification lives in the project, not in the renderer: it is
//! something the project *declares*, and the renderer is the thing that obeys
//! it. Putting it here is what lets `render` depend on `project` without
//! `project` depending on `render`.
//!
//! A specification is complete on its own. Given the project and the
//! specification, and nothing else — no flags, no environment, no clock — the
//! renderer produces the same bytes on every machine. That property is what
//! makes `shaipe render` usable as a CI check.

use std::fmt;

use crate::project::palette::Rgba;

/// What to encode a render as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// A raster PNG.
    #[default]
    Png,
    /// The isolated variant, as a standalone SVG document.
    Svg,
}

impl Format {
    /// The file extension, without the dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Svg => "svg",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "png" => Ok(Self::Png),
            "svg" => Ok(Self::Svg),
            other => Err(format!("unknown format `{other}`, expected `png` or `svg`")),
        }
    }
}

/// What the variant is drawn on top of.
///
/// A distinct type rather than an `Option<Rgba>`, because "transparent" is a
/// deliberate choice a project makes and reads very differently from "not
/// specified" when it appears in a diff or a TUI pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Background {
    /// Nothing: the canvas keeps its alpha.
    #[default]
    Transparent,
    /// A flat colour.
    Colour(Rgba),
}

impl Background {
    /// The colour to clear the canvas with.
    #[must_use]
    pub const fn as_rgba(self) -> Rgba {
        match self {
            Self::Transparent => Rgba::TRANSPARENT,
            Self::Colour(colour) => colour,
        }
    }
}

impl fmt::Display for Background {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transparent => f.write_str("transparent"),
            Self::Colour(colour) => write!(f, "{colour}"),
        }
    }
}

impl std::str::FromStr for Background {
    type Err = crate::Error;

    fn from_str(s: &str) -> crate::Result<Self> {
        match s {
            "transparent" | "none" => Ok(Self::Transparent),
            colour => colour.parse().map(Self::Colour),
        }
    }
}

/// One asset to produce from one variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSpec {
    /// The output file's stem, and how the project refers to this asset.
    pub name: String,
    /// Which variant to draw.
    pub variant: String,
    /// Canvas width, in pixels.
    pub width: u32,
    /// Canvas height, in pixels.
    pub height: u32,
    /// How to encode it.
    pub format: Format,
    /// What to draw it on.
    pub background: Background,
}

impl RenderSpec {
    /// A square PNG of a variant, on a transparent background.
    ///
    /// The shape of the overwhelming majority of specifications, and what
    /// `shaipe render --variant x --width n` builds.
    #[must_use]
    pub fn square(name: impl Into<String>, variant: impl Into<String>, size: u32) -> Self {
        Self {
            name: name.into(),
            variant: variant.into(),
            width: size,
            height: size,
            format: Format::default(),
            background: Background::default(),
        }
    }

    /// The file name this specification writes, including its extension.
    #[must_use]
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.name, self.format.extension())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_specification_names_its_own_output_file() {
        let mut spec = RenderSpec::square("favicon-32", "icon", 32);
        assert_eq!(spec.file_name(), "favicon-32.png");

        spec.format = Format::Svg;
        assert_eq!(spec.file_name(), "favicon-32.svg");
    }

    #[test]
    fn transparent_and_an_explicit_colour_are_distinguishable_backgrounds() {
        assert_eq!(
            "transparent".parse::<Background>().unwrap(),
            Background::Transparent
        );
        assert_eq!(
            "#18181b".parse::<Background>().unwrap(),
            Background::Colour(Rgba::new(0x18, 0x18, 0x1b, 0xff))
        );
        assert!(Background::Transparent.as_rgba().is_transparent());
    }

    #[test]
    fn an_unparseable_background_is_rejected_rather_than_treated_as_transparent() {
        // Silently falling back to transparent would produce a plausible-looking
        // asset with the wrong background, which is worse than failing.
        assert!("chartreuse".parse::<Background>().is_err());
    }
}
