//! Measuring a raster reference into vector paths, rather than describing it.
//!
//! An LLM writing `<path d="...">` from a prompt and an image is doing text
//! generation: it can name the shapes present ("hexagon, shackle, keyhole")
//! and estimate rough placement, but it has no mechanism for measuring an
//! actual corner radius or curve tangent from pixels — those numbers are
//! guessed toward "plausible SVG for this kind of icon", not fitted to the
//! image. This module is the other kind of tool: [`trace`] walks pixel
//! contours into cubic bezier paths algorithmically. See ADR 014 for the
//! decision and why an algorithm rather than a second model call.
//!
//! Self-contained and deterministic: bytes in, an SVG string out, no
//! knowledge of `Project`, tools or MCP — mirroring [`crate::render`]'s
//! isolation from the terminal. Given the same bytes and the same
//! [`TraceOptions`] this produces the same output, on any machine — the same
//! property invariant 1 (`AGENTS.md`) already requires of the renderer, now
//! proven for tracing by
//! `tracing_the_same_image_twice_produces_identical_bytes` below.

use std::path::Path;

use image::GenericImageView;
use vtracer::{Color as TraceColor, ColorImage, Config, Preset};

use crate::error::{Error, Result};

/// How a reference image is traced.
///
/// Deliberately narrow: only the two knobs needed to separate foreground
/// from background on an arbitrary crop — not vtracer's full surface
/// (colour/photo modes, palettes, mosaic compositing). Brand marks are
/// single-colour; extend this if real usage needs more, the way `set_` was
/// "anticipated but not used" until it was (ADR-007).
#[derive(Debug, Clone, Copy, Default)]
pub struct TraceOptions {
    /// 0..=255 binary cutoff; pixels darker than this are traced as
    /// foreground. `None` uses vtracer's own default (128).
    pub threshold: Option<u8>,
    /// Set when the reference is light artwork on a dark background, so
    /// light pixels are traced as the foreground instead of dark ones.
    ///
    /// Also chooses what a transparent pixel becomes: composited onto white
    /// when `false` (a transparent pixel reads as background, the same as a
    /// dark-on-light mark), onto black when `true` (so it still reads as
    /// background once colours are inverted). The binary frontend judges
    /// darkness from RGB alone and does not see alpha at all, so a
    /// transparent pixel left as-is would be classified by whatever colour
    /// its RGB happens to hold underneath the transparency, not by what it
    /// looks like — this is what stops that from being a silent miscount.
    pub invert: bool,
}

/// Decode `bytes` (PNG/JPEG/GIF/WEBP/BMP — the formats
/// [`crate::tools`]'s `get_reference_image` already recognises) and trace it
/// to a single-colour SVG silhouette: one shape, holes cut by reversed
/// winding, no `fill`/`stroke` baked in beyond whatever solid colour the
/// frontend assigns — recolouring is the caller's decision, via
/// `write_variant`/`write_svg`, the same as every other mark in a project.
///
/// `path` is only used to name the reference in an error; it is never read.
///
/// # Errors
///
/// Returns [`Error::Decode`] if `bytes` is not a recognisable image, or
/// [`Error::EmptyTrace`] if nothing in it was darker (or, inverted, lighter)
/// than the threshold.
pub fn trace(path: &Path, bytes: &[u8], options: TraceOptions) -> Result<String> {
    let decoded = image::load_from_memory(bytes).map_err(|source| Error::Decode {
        path: path.to_path_buf(),
        source,
    })?;

    let (width, height) = decoded.dimensions();
    let mut pixels = decoded.to_rgba8().into_raw();
    flatten_transparency(&mut pixels, options.invert);
    if options.invert {
        invert_rgb(&mut pixels);
    }

    let image = ColorImage {
        pixels,
        width: width as usize,
        height: height as usize,
    };

    let mut config = Config::from_preset(Preset::Bw);
    if let Some(threshold) = options.threshold {
        config.binary_threshold = threshold;
    }

    // `Config::build` only fails for a config this module never produces
    // (`Hierarchical::Cutout` needs an area frontend can't feed it — see
    // vtracer's own `Compositing::Mosaic` docs); a config error here would be
    // a bug in this function, not something a caller passed in, so it is not
    // its own `Error` variant. Same for `Pipeline::to_svg`'s error: the only
    // failure it can return beyond a bad config is `Cancelled`, and nothing
    // here ever creates a `CancelToken` that gets tripped.
    let svg = config
        .build()
        .and_then(|pipeline| pipeline.to_svg(&image))
        .map_err(|source| Error::Trace {
            path: path.to_path_buf(),
            reason: source.to_string(),
        })?;

    if !svg.contains("<path") {
        return Err(Error::EmptyTrace {
            path: path.to_path_buf(),
        });
    }

    Ok(svg)
}

/// Composite `pixels` (RGBA8, 4 bytes per pixel) onto an opaque backdrop —
/// white when `invert` is `false`, black when it is `true` — so a
/// transparent pixel reads as background rather than as whatever colour its
/// RGB happens to hold underneath the transparency. See [`TraceOptions::invert`].
fn flatten_transparency(pixels: &mut [u8], invert: bool) {
    let backdrop: u32 = if invert { 0 } else { 255 };
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            let source = u32::from(*channel);
            *channel = ((source * alpha + backdrop * (255 - alpha)) / 255) as u8;
        }
        pixel[3] = 255;
    }
}

/// Invert the RGB channels of `pixels` (RGBA8, 4 bytes per pixel) in place,
/// leaving alpha untouched. Used for [`TraceOptions::invert`] so light
/// artwork on a dark background is traced the same way dark artwork on a
/// light one is — the binary frontend only ever treats *dark* pixels as
/// foreground.
fn invert_rgb(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[0] = 255 - pixel[0];
        pixel[1] = 255 - pixel[1];
        pixel[2] = 255 - pixel[2];
    }
}

// Referenced only to keep `vtracer::Color` — re-exported for callers who want
// to build a fixed palette later (out of scope for v1, see ADR 014) — from
// being an unused import lint until that day comes.
#[allow(dead_code)]
type _KeepColorInScopeUntilPaletteSupportArrives = TraceColor;

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny synthetic PNG: a black ring (a filled circle with a smaller
    /// circle cut from its centre), `size` pixels square. `background`
    /// chooses what surrounds the ring — `None` for fully transparent, or an
    /// explicit opaque colour to compare against. Built at test time rather
    /// than checked in as a fixture file, so the exact pixels are visible in
    /// the test that depends on them.
    fn ring_png(size: u32, background: Option<[u8; 3]>) -> Vec<u8> {
        let mut img = image::RgbaImage::new(size, size);
        let center = f64::from(size) / 2.0;
        let outer = center - 4.0;
        let inner = outer / 2.5;
        for y in 0..size {
            for x in 0..size {
                let dx = f64::from(x) - center;
                let dy = f64::from(y) - center;
                let d = (dx * dx + dy * dy).sqrt();
                let pixel = if d <= outer && d >= inner {
                    image::Rgba([0, 0, 0, 255])
                } else {
                    match background {
                        Some([r, g, b]) => image::Rgba([r, g, b, 255]),
                        None => image::Rgba([0, 0, 0, 0]),
                    }
                };
                img.put_pixel(x, y, pixel);
            }
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding a freshly-built test image never fails");
        bytes
    }

    fn light_on_dark_png(size: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::new(size, size);
        let center = f64::from(size) / 2.0;
        let radius = center - 4.0;
        for y in 0..size {
            for x in 0..size {
                let dx = f64::from(x) - center;
                let dy = f64::from(y) - center;
                let inside = (dx * dx + dy * dy).sqrt() <= radius;
                let pixel = if inside {
                    image::Rgba([255, 255, 255, 255])
                } else {
                    image::Rgba([20, 20, 20, 255])
                };
                img.put_pixel(x, y, pixel);
            }
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding a freshly-built test image never fails");
        bytes
    }

    #[test]
    fn tracing_the_same_image_twice_produces_identical_bytes() {
        let bytes = ring_png(64, None);
        let path = Path::new("ring.png");

        let first = trace(path, &bytes, TraceOptions::default()).unwrap();
        let second = trace(path, &bytes, TraceOptions::default()).unwrap();

        assert_eq!(first, second, "tracing must be deterministic");
    }

    #[test]
    fn a_ring_traces_to_a_single_path_with_a_hole() {
        let bytes = ring_png(64, None);
        let svg = trace(Path::new("ring.png"), &bytes, TraceOptions::default()).unwrap();

        // One region (the ring), fitted as one path whose fill-rule cuts the
        // inner circle as a hole, the same shape lock.svg turned out to be.
        assert_eq!(svg.matches("<path").count(), 1, "{svg}");
    }

    #[test]
    fn a_transparent_background_traces_the_same_as_the_same_ring_on_white() {
        // Every pixel outside the ring is fully transparent with RGB (0,0,0)
        // — black, and darker than the default threshold. If the transparent
        // version were not flattened onto white first, it would trace as a
        // solid disc (the whole canvas is "dark"), not a ring — a visibly
        // different result from tracing the same ring already opaque on
        // white. Flattened correctly, the two must be identical: flattening
        // onto white is exactly what an unflattened white background already
        // looks like.
        let transparent = ring_png(64, None);
        let opaque_white = ring_png(64, Some([255, 255, 255]));

        let traced_transparent =
            trace(Path::new("ring.png"), &transparent, TraceOptions::default()).unwrap();
        let traced_white = trace(
            Path::new("ring.png"),
            &opaque_white,
            TraceOptions::default(),
        )
        .unwrap();

        assert_eq!(traced_transparent, traced_white);
    }

    #[test]
    fn invert_traces_light_artwork_on_a_dark_background() {
        let bytes = light_on_dark_png(64);

        let not_inverted = trace(Path::new("disc.png"), &bytes, TraceOptions::default()).unwrap();
        let inverted = trace(
            Path::new("disc.png"),
            &bytes,
            TraceOptions {
                invert: true,
                ..TraceOptions::default()
            },
        )
        .unwrap();

        // Without inverting, the dark background is what reads as
        // foreground (nearly the whole canvas); inverted, the light disc
        // does. Different foregrounds fit to different path data.
        assert_ne!(not_inverted, inverted);
        assert!(inverted.contains("<path"), "{inverted}");
    }

    #[test]
    fn threshold_changes_what_counts_as_foreground() {
        // A mid-gray disc: darker than a high threshold, lighter than a low
        // one.
        let size = 64;
        let mut img = image::RgbaImage::new(size, size);
        let center = f64::from(size) / 2.0;
        let radius = center - 4.0;
        for y in 0..size {
            for x in 0..size {
                let dx = f64::from(x) - center;
                let dy = f64::from(y) - center;
                let inside = (dx * dx + dy * dy).sqrt() <= radius;
                let pixel = if inside {
                    image::Rgba([128, 128, 128, 255])
                } else {
                    image::Rgba([255, 255, 255, 255])
                };
                img.put_pixel(x, y, pixel);
            }
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

        let low = trace(
            Path::new("gray.png"),
            &bytes,
            TraceOptions {
                threshold: Some(50),
                ..TraceOptions::default()
            },
        );
        let high = trace(
            Path::new("gray.png"),
            &bytes,
            TraceOptions {
                threshold: Some(200),
                ..TraceOptions::default()
            },
        )
        .unwrap();

        // Below the gray disc's own intensity (128), nothing is foreground.
        assert!(matches!(low, Err(Error::EmptyTrace { .. })), "{low:?}");
        assert!(high.contains("<path"), "{high}");
    }

    #[test]
    fn a_blank_image_reports_that_nothing_was_traced() {
        let size = 32;
        let img = image::RgbaImage::from_pixel(size, size, image::Rgba([255, 255, 255, 255]));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

        let error = trace(Path::new("blank.png"), &bytes, TraceOptions::default()).unwrap_err();

        assert!(matches!(error, Error::EmptyTrace { .. }), "{error:?}");
    }

    #[test]
    fn bytes_that_are_not_an_image_report_why() {
        let error = trace(
            Path::new("not-an-image"),
            b"not a png",
            TraceOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(error, Error::Decode { .. }), "{error:?}");
    }
}
