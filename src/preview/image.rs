//! Pixels, decoupled from how they were produced.
//!
//! The preview layer never sees a project, a variant or a render
//! specification — only this. That is what keeps a terminal graphics protocol
//! from acquiring an opinion about SVG.

use crate::error::{Error, Result};
use crate::render::RenderedAsset;

/// An 8-bit RGBA image in memory, straight alpha, row-major.
///
/// Straight rather than premultiplied because both preview backends need
/// straight values: Kitty transmits PNG, and half-blocks composite against a
/// background themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Image {
    /// Build an image from raw RGBA bytes.
    ///
    /// # Panics
    ///
    /// If `pixels` is not exactly `width * height * 4` bytes long.
    #[must_use]
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(
            pixels.len(),
            width as usize * height as usize * 4,
            "an RGBA image must carry exactly four bytes per pixel"
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Decode a rendered PNG asset into pixels.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnpreviewableAsset`] for an asset that is not a raster
    /// image — an SVG render has no pixels until something rasterises it, and
    /// silently showing nothing would look like a rendering bug.
    pub fn from_asset(asset: &RenderedAsset) -> Result<Self> {
        let pixmap =
            tiny_skia::Pixmap::decode_png(&asset.bytes).map_err(|_| Error::UnpreviewableAsset {
                spec: asset.spec.name.clone(),
                format: asset.spec.format.to_string(),
            })?;

        // `tiny-skia` stores premultiplied pixels; `demultiply` recovers the
        // colour of a partially transparent pixel, which is exactly what gets
        // lost if the raw buffer is used as-is.
        let mut pixels = Vec::with_capacity(pixmap.pixels().len() * 4);
        for pixel in pixmap.pixels() {
            let colour = pixel.demultiply();
            pixels.extend_from_slice(&[
                colour.red(),
                colour.green(),
                colour.blue(),
                colour.alpha(),
            ]);
        }

        Ok(Self::from_rgba(pixmap.width(), pixmap.height(), pixels))
    }

    /// Width, in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height, in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The raw RGBA bytes.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The pixel at `(x, y)`, or fully transparent when out of bounds.
    ///
    /// Out of bounds returns transparent rather than panicking because the
    /// half-block backend samples a grid that may overhang the image, and
    /// clamping every read at the call site would be noise.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        ]
    }

    /// Encode as a PNG.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Encode`] if the pixmap will not encode.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut pixmap =
            tiny_skia::Pixmap::new(self.width, self.height).ok_or(Error::InvalidSize {
                width: self.width,
                height: self.height,
            })?;
        pixmap.data_mut().copy_from_slice(&self.pixels);
        // The buffer just written is straight alpha, and `encode_png` expects
        // exactly that, so no premultiplication happens on this path.
        pixmap.encode_png().map_err(|source| Error::Encode {
            spec: "preview".to_owned(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;
    use crate::project::RenderSpec;

    #[test]
    fn an_asset_decodes_to_pixels_of_the_same_size() {
        let project = fixtures::project();
        let spec = RenderSpec::square("probe", "icon", 16);
        let asset = crate::render::render(&project, &spec).unwrap();
        let image = Image::from_asset(&asset).unwrap();

        assert_eq!((image.width(), image.height()), (16, 16));
        assert_eq!(image.pixel(8, 8), [0xf0, 0x50, 0x32, 0xff]);
    }

    #[test]
    fn a_transparent_pixel_keeps_its_colour_through_demultiplication() {
        // A premultiplied transparent pixel is all zeroes; reading the raw
        // buffer instead of demultiplying turns every soft edge into black.
        let image = Image::from_rgba(1, 1, vec![0xf0, 0x50, 0x32, 0x80]);
        let png = image.to_png().unwrap();
        let decoded = tiny_skia::Pixmap::decode_png(&png).unwrap();
        let pixel = decoded.pixel(0, 0).unwrap().demultiply();

        assert_eq!(pixel.alpha(), 0x80);
        assert!(pixel.red() > 0xe0, "red was {}", pixel.red());
    }

    #[test]
    fn sampling_outside_an_image_is_transparent_rather_than_a_panic() {
        let image = Image::from_rgba(1, 1, vec![0xff, 0xff, 0xff, 0xff]);
        assert_eq!(image.pixel(9, 9), [0, 0, 0, 0]);
    }

    #[test]
    fn an_svg_asset_cannot_be_previewed_and_says_so() {
        let project = fixtures::project();
        let mut spec = RenderSpec::square("probe", "icon", 16);
        spec.format = crate::project::Format::Svg;
        let asset = crate::render::render(&project, &spec).unwrap();

        assert!(matches!(
            Image::from_asset(&asset).unwrap_err(),
            Error::UnpreviewableAsset { .. }
        ));
    }
}
