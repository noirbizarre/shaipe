//! Pixels, decoupled from how they were produced.
//!
//! The preview layer never sees a project, a variant or a render
//! specification — only this. That is what keeps a terminal graphics protocol
//! from acquiring an opinion about SVG.

use crate::error::{Error, Result};

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

    /// Decode a PNG into pixels.
    ///
    /// Takes bytes rather than a [`crate::render::RenderedAsset`] on purpose:
    /// the preview layer knows about images and nothing else, so that a
    /// terminal graphics protocol can never acquire an opinion about SVG.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotAPng`] if the bytes are not a PNG.
    pub fn from_png(bytes: &[u8]) -> Result<Self> {
        let pixmap = tiny_skia::Pixmap::decode_png(bytes).map_err(|_| Error::NotAPng)?;

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

    /// Convert to the image type `ratatui-image` speaks.
    ///
    /// Confined to this module on purpose: it is the single point at which
    /// the preview backend's vocabulary enters, and keeping it here is what
    /// lets everything else in the crate deal only in [`Image`].
    pub(crate) fn to_dynamic(&self) -> ::image::DynamicImage {
        let buffer = ::image::RgbaImage::from_raw(self.width, self.height, self.pixels.clone())
            .expect("an `Image` always carries exactly four bytes per pixel");
        ::image::DynamicImage::ImageRgba8(buffer)
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

    /// A PNG of a solid colour, built without going near a project.
    ///
    /// The preview layer must be testable from pixels alone; a test that
    /// rendered an SVG to get its input would quietly reintroduce the
    /// dependency this module exists to avoid.
    fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut pixmap = tiny_skia::Pixmap::new(width, height).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(
            rgba[0], rgba[1], rgba[2], rgba[3],
        ));
        pixmap.encode_png().unwrap()
    }

    #[test]
    fn a_png_decodes_to_pixels_of_the_same_size() {
        let image = Image::from_png(&png(16, 8, [0xf0, 0x50, 0x32, 0xff])).unwrap();

        assert_eq!((image.width(), image.height()), (16, 8));
        assert_eq!(image.pixel(8, 4), [0xf0, 0x50, 0x32, 0xff]);
    }

    #[test]
    fn a_transparent_pixel_keeps_its_colour_through_demultiplication() {
        // A premultiplied transparent pixel is all zeroes; reading the raw
        // buffer instead of demultiplying turns every soft edge into black.
        let image = Image::from_rgba(1, 1, vec![0xf0, 0x50, 0x32, 0x80]);
        let decoded = Image::from_png(&image.to_png().unwrap()).unwrap();
        let [red, _, _, alpha] = decoded.pixel(0, 0);

        assert_eq!(alpha, 0x80);
        assert!(red > 0xe0, "red was {red:#x}");
    }

    #[test]
    fn sampling_outside_an_image_is_transparent_rather_than_a_panic() {
        let image = Image::from_rgba(1, 1, vec![0xff, 0xff, 0xff, 0xff]);
        assert_eq!(image.pixel(9, 9), [0, 0, 0, 0]);
    }

    #[test]
    fn converting_for_the_preview_backend_preserves_size_and_colour() {
        let image = Image::from_rgba(2, 1, vec![0xf0, 0x50, 0x32, 0xff, 0, 0, 0, 0]);
        let dynamic = image.to_dynamic();

        assert_eq!((::image::GenericImageView::dimensions(&dynamic)), (2, 1));
        let rgba = dynamic.to_rgba8();
        assert_eq!(rgba.get_pixel(0, 0).0, [0xf0, 0x50, 0x32, 0xff]);
        assert_eq!(rgba.get_pixel(1, 0).0, [0, 0, 0, 0]);
    }

    #[test]
    fn bytes_that_are_not_a_png_are_refused() {
        assert!(matches!(
            Image::from_png(b"<svg/>").unwrap_err(),
            Error::NotAPng
        ));
    }
}
