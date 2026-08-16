//! The half-block fallback.
//!
//! Each cell renders `▀` with the foreground set to the pixel above and the
//! background to the pixel below, which buys two vertical pixels per cell in a
//! terminal that can do nothing but colour text. That is every terminal, which
//! is the point: this backend has no detection, no capability negotiation and
//! no way to fail, so there is always something to fall back to.
//!
//! Unlike [`crate::preview::kitty`] this draws into ratatui's buffer, so the
//! [`Preview`] implementation is a thin shim over a [`Widget`] and the widget
//! is what the TUI actually uses.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

use crate::preview::{Image, Preview};

/// The half-block backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct Blocks;

impl Preview for Blocks {
    fn name(&self) -> &'static str {
        "blocks"
    }

    fn draw(&mut self, _image: &Image, _area: Rect) -> std::io::Result<()> {
        // Nothing to write out of band: this backend's pixels go through
        // ratatui's buffer, via `HalfBlocks`. The trait is still implemented so
        // that `detect` can return either backend behind the same object.
        Ok(())
    }
}

/// An [`Image`] as a ratatui widget.
///
/// Fits the image into the area preserving its aspect ratio, treating a cell
/// as two pixels tall. Anything the image does not cover is left alone, so the
/// surrounding block and background show through.
#[derive(Debug, Clone, Copy)]
pub struct HalfBlocks<'a> {
    image: &'a Image,
}

impl<'a> HalfBlocks<'a> {
    /// Draw an image with half-blocks.
    #[must_use]
    pub const fn new(image: &'a Image) -> Self {
        Self { image }
    }
}

/// Blend a straight-alpha pixel onto the terminal's background.
///
/// A terminal cell has no alpha, so transparency has to be resolved to
/// something. `None` means "leave the cell as it is", which lets the pane's
/// own background show through instead of forcing black — the difference
/// between previewing a transparent logo and previewing a black rectangle.
fn colour([r, g, b, a]: [u8; 4]) -> Option<Color> {
    match a {
        0 => None,
        255 => Some(Color::Rgb(r, g, b)),
        // Composited against the terminal's default background, which is
        // unknowable, so it is approximated by scaling towards black. Wrong on
        // a light theme, but only for partially transparent pixels, and only
        // in a preview.
        alpha => {
            let scale = |channel: u8| ((u16::from(channel) * u16::from(alpha)) / 255) as u8;
            Some(Color::Rgb(scale(r), scale(g), scale(b)))
        }
    }
}

impl Widget for HalfBlocks<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.image.width() == 0 {
            return;
        }

        // A cell is two pixels tall, so the area's pixel height is doubled
        // before the fit is computed. Forgetting this is what makes half-block
        // previews come out squashed.
        let available_width = f64::from(area.width);
        let available_height = f64::from(area.height) * 2.0;
        let scale = (available_width / f64::from(self.image.width()))
            .min(available_height / f64::from(self.image.height()));

        let width = ((f64::from(self.image.width()) * scale).round() as u16).min(area.width);
        let rows = ((f64::from(self.image.height()) * scale / 2.0).round() as u16)
            .clamp(0, area.height)
            .max(1);

        // Centred, because an off-centre preview reads as a rendering bug.
        let left = area.x + (area.width - width) / 2;
        let top = area.y + (area.height - rows) / 2;

        for row in 0..rows {
            for column in 0..width {
                let x = (f64::from(column) / scale) as u32;
                let upper = (f64::from(row) * 2.0 / scale) as u32;
                let lower = ((f64::from(row) * 2.0 + 1.0) / scale) as u32;

                let cell = &mut buffer[(left + column, top + row)];
                cell.set_char('▀');
                if let Some(fg) = colour(self.image.pixel(x, upper)) {
                    cell.set_fg(fg);
                }
                if let Some(bg) = colour(self.image.pixel(x, lower)) {
                    cell.set_bg(bg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// A solid image of a single colour.
    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Image {
        Image::from_rgba(
            width,
            height,
            rgba.iter()
                .copied()
                .cycle()
                .take((width * height * 4) as usize)
                .collect(),
        )
    }

    #[test]
    fn a_square_image_in_a_wide_area_is_centred_rather_than_stretched() {
        let image = solid(8, 8, [0xff, 0x00, 0x00, 0xff]);
        let area = Rect::new(0, 0, 40, 4);
        let mut buffer = Buffer::empty(area);
        HalfBlocks::new(&image).render(area, &mut buffer);

        // Eight pixels tall in four rows of two: the image occupies eight
        // columns, centred in forty.
        assert_eq!(
            buffer[(0, 0)].symbol(),
            " ",
            "left edge should be untouched"
        );
        assert_eq!(buffer[(20, 2)].symbol(), "▀");
        assert_eq!(buffer[(20, 2)].fg, Color::Rgb(0xff, 0, 0));
    }

    #[test]
    fn a_fully_transparent_image_leaves_the_cells_colours_alone() {
        // Otherwise a logo with a transparent background previews as a solid
        // black rectangle, which looks like a broken render.
        let image = solid(4, 4, [0, 0, 0, 0]);
        let area = Rect::new(0, 0, 10, 4);
        let mut buffer = Buffer::empty(area);
        HalfBlocks::new(&image).render(area, &mut buffer);

        assert_eq!(buffer[(5, 2)].fg, Color::Reset);
        assert_eq!(buffer[(5, 2)].bg, Color::Reset);
    }

    #[test]
    fn rendering_into_a_zero_sized_area_does_nothing_instead_of_panicking() {
        // Panes get squeezed to nothing while a terminal is being resized.
        let image = solid(4, 4, [0xff, 0xff, 0xff, 0xff]);
        let area = Rect::new(0, 0, 0, 0);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 10));
        HalfBlocks::new(&image).render(area, &mut buffer);
    }

    #[test]
    fn the_fallback_backend_never_fails() {
        // It is what everything else falls back *to*; if it could fail there
        // would be nothing left.
        let image = solid(2, 2, [1, 2, 3, 4]);
        assert!(Blocks.draw(&image, Rect::new(0, 0, 4, 4)).is_ok());
        assert!(Blocks.clear().is_ok());
    }
}
