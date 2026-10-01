//! Comparing a reference against a rendered variant — pixels, not opinions.
//!
//! [`crate::analysis`] measures one image; this measures the *difference*
//! between two, reusing exactly the background/foreground primitives ADR 018
//! built for that purpose ("`#5`'s comparison tool needs a
//! background/foreground split and a bounding box to align a reference
//! against a render" — ADR 018's own words, written before this existed).
//! See ADR 019 for the rest of the decision.
//!
//! # Canvas
//!
//! [`compare`] renders nothing itself — see [`crate::render`] for that — and
//! takes two already-encoded images. The canvas the two are compared on is
//! decided by the caller, not by this module: [`crate::tools::builtin`]'s
//! `compare_reference` tool always renders the variant at the reference's
//! own pixel dimensions, so the two arrive here already the same size and
//! nothing is resampled. [`compare`] itself only refuses a mismatch
//! ([`Error::CompareDimensionMismatch`]) rather than silently resizing
//! either one — a caller choosing not to align two different-sized images is
//! a caller error, not a decision this module should make quietly.
//!
//! # What is measured, and by what
//!
//! Every signal is named separately — never folded into one score, per the
//! issue this closes (#5) and ADR 018's own precedent. Simple, deterministic
//! pixel arithmetic (mask overlap, bounding box and centroid offsets, area
//! difference, MAE/MSE/RMSE, edge overlap) is hand-rolled here, the same way
//! `analysis` and `vectorize` do their own measurement. Perceptual
//! similarity is not: SSIM's maths is easy to get subtly wrong, so
//! [`PerceptualSimilarity`] borrows a well-tested crate
//! (`image-compare`'s `rgba_hybrid_compare`) instead of reimplementing it,
//! and its per-pixel diff image becomes [`ComparisonImages::difference`] for
//! free.

mod appearance;
mod typography;

use std::path::Path;

use image::{DynamicImage, RgbaImage};
use serde::Serialize;

use crate::analysis::{self, Background, BoundingBox, Centroid, Classified, Dimensions};
use crate::error::{Error, Result};

pub use appearance::{AlphaComparison, AppearanceComparison, RegionComparison};
pub use typography::{LineComparison, TypographyComparison, declared_text_findings};

/// A per-channel difference below this (out of 255) is not counted as
/// "changed" for [`PixelError::changed_pixel_fraction`] — real
/// re-encoding/anti-aliasing noise rarely lands exactly at 0.
const PIXEL_CHANGE_THRESHOLD: u8 = 8;

/// A Sobel gradient magnitude at or above this counts as an edge pixel.
/// Chosen the same way [`analysis`]'s own thresholds were: wide enough that
/// anti-aliasing does not vanish the edges of a real logo, narrow enough
/// that a flat fill does not register as one everywhere.
const EDGE_GRADIENT_THRESHOLD: f32 = 64.0;

/// Everything measured comparing a reference against a rendered variant.
/// Every field is a named, independent fact — never combined into one
/// opaque score.
#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    /// The shared canvas both images were compared on — the reference's own
    /// pixel dimensions; the render was produced to match this exactly.
    pub canvas: Dimensions,
    /// What the reference's border says about its background. See
    /// [`analysis::Background`].
    pub reference_background: Background,
    /// The render's own border sample. Ordinarily fully transparent, since
    /// `compare_reference` always renders onto a transparent canvas — see
    /// this same caveat on [`analysis::Background`] when it is not.
    pub render_background: Background,
    /// Overlap of the two foreground masks.
    pub foreground: MaskComparison,
    /// Overlap of the two images' edge masks — a Sobel gradient magnitude,
    /// thresholded. Answers a different question than `foreground`: two
    /// silhouettes can share almost every foreground pixel while their
    /// outlines sit at different sub-shapes, or vice versa for a translated
    /// but otherwise identical mark.
    pub edges: MaskComparison,
    /// Bounding-box position and size differences.
    pub bounding_box: BoundingBoxComparison,
    /// Centroid position differences.
    pub centroid: CentroidComparison,
    /// Whole-canvas pixel error.
    pub pixel_error: PixelError,
    /// SSIM-based perceptual similarity.
    pub perceptual_similarity: PerceptualSimilarity,
    /// How the two are filled: gradients, colours, opacity and strokes,
    /// region against region. Reported apart from the geometry above so a
    /// render that is the right shape with the wrong fill says so.
    pub appearance: AppearanceComparison,
    /// Lettering, line against line: baseline, letter height, width, spacing
    /// and colour. Left out when neither image has any, so a comparison of
    /// artwork without text reads as it always did.
    #[serde(skip_serializing_if = "TypographyComparison::is_empty")]
    pub typography: TypographyComparison,
}

/// Overlap of two boolean masks of the same canvas — shared shape for
/// [`Comparison::foreground`] and [`Comparison::edges`].
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MaskComparison {
    /// Share of every canvas pixel the reference marks.
    pub reference_fraction: f64,
    /// Share of every canvas pixel the render marks.
    pub render_fraction: f64,
    /// `|reference_fraction - render_fraction|`.
    pub absolute_difference: f64,
    /// Intersection over union of the two masks. `1.0` when neither mask
    /// marks anything at all — nothing to disagree about — rather than the
    /// `0 / 0` that would otherwise produce, the same convention
    /// [`analysis`]'s own symmetry score uses.
    pub intersection_over_union: f64,
}

/// A 2D displacement, in whatever unit the field it appears on names.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Offset {
    /// Horizontal component.
    pub dx: f64,
    /// Vertical component.
    pub dy: f64,
}

/// Bounding-box differences between the reference's foreground and the
/// render's. Every field but the boxes themselves is `None` when either mask
/// has no foreground pixel at all — there is nothing to offset from.
#[derive(Debug, Clone, Serialize)]
pub struct BoundingBoxComparison {
    /// The reference's foreground bounding box.
    pub reference: Option<BoundingBox>,
    /// The render's foreground bounding box.
    pub render: Option<BoundingBox>,
    /// The render's box centre minus the reference's, in pixels.
    pub offset: Option<Offset>,
    /// `offset`, normalised by canvas width/height.
    pub offset_fraction: Option<Offset>,
    /// `render.width / reference.width`.
    pub width_ratio: Option<f64>,
    /// `render.height / reference.height`.
    pub height_ratio: Option<f64>,
}

/// Centroid differences between the reference's foreground and the
/// render's. `None` fields for the same reason as
/// [`BoundingBoxComparison`]'s.
#[derive(Debug, Clone, Serialize)]
pub struct CentroidComparison {
    /// The reference foreground's mean pixel position.
    pub reference: Option<Centroid>,
    /// The render foreground's mean pixel position.
    pub render: Option<Centroid>,
    /// The render's centroid minus the reference's, in pixels.
    pub offset: Option<Offset>,
    /// `offset`, normalised by canvas width/height.
    pub offset_fraction: Option<Offset>,
}

/// Per-channel mean absolute error, normalised to `0.0..1.0`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ChannelErrors {
    /// Red channel.
    pub r: f64,
    /// Green channel.
    pub g: f64,
    /// Blue channel.
    pub b: f64,
    /// Alpha channel.
    pub a: f64,
}

/// Whole-canvas pixel error between the reference and the render, over every
/// pixel and every channel — background included, not only the foreground.
/// Every field is normalised so `0.0..1.0` — 0 identical, 1 the maximum
/// possible per-channel difference — is comparable across images regardless
/// of size.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PixelError {
    /// Share of pixels with at least one channel differing by
    /// [`PIXEL_CHANGE_THRESHOLD`] (out of 255) or more.
    pub changed_pixel_fraction: f64,
    /// Mean absolute per-channel difference, over every channel of every
    /// pixel.
    pub mean_absolute_error: f64,
    /// Mean squared per-channel difference.
    pub mean_squared_error: f64,
    /// `mean_squared_error.sqrt()`.
    pub root_mean_squared_error: f64,
    /// The single largest per-channel difference found anywhere on the
    /// canvas.
    pub max_absolute_error: f64,
    /// Mean absolute error, broken out by channel — a colour-only
    /// discrepancy shows up here even when `mean_absolute_error` looks small
    /// against a mostly-transparent canvas.
    pub per_channel_mean_absolute_error: ChannelErrors,
}

/// An SSIM-based perceptual similarity score, from the `image-compare`
/// crate rather than a hand-rolled implementation — see the module doc
/// comment and ADR 019.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PerceptualSimilarity {
    /// Which algorithm produced [`Self::score`], named so a future second
    /// algorithm does not have to guess which one an old report used.
    pub algorithm: &'static str,
    /// `1.0` identical, lower is more different. Can go negative for a
    /// pathological negative covariance; see `image-compare`'s own
    /// documentation of `Similarity::score`.
    pub score: f64,
}

/// PNG bytes worth looking at, produced alongside [`Comparison`]. Not
/// [`Serialize`] — these are handed to the tool layer to wrap as images, not
/// embedded in the JSON report.
#[derive(Debug)]
pub struct ComparisonImages {
    /// Reference-only foreground in one colour, render-only in another,
    /// agreement in a third, over a transparent canvas — where the two
    /// silhouettes agree and disagree, at a glance.
    pub overlay: Vec<u8>,
    /// `image-compare`'s own per-pixel diff, colourised — see
    /// [`PerceptualSimilarity`].
    pub difference: Vec<u8>,
}

/// Compare a reference image against a rendered variant.
///
/// Self-contained like [`analysis::analyze`]: bytes in (twice), a typed
/// report and two images out, no knowledge of `Project`, tools or MCP.
/// `reference_path`/`render_path` are only used to name either image in a
/// decode error; neither is read.
///
/// # Errors
///
/// Returns [`Error::AnalysisDecode`] if either image will not decode,
/// [`Error::CompareDimensionMismatch`] if they decode to different pixel
/// dimensions, and [`Error::PerceptualSimilarity`] if the perceptual
/// comparison itself fails.
pub fn compare(
    reference_path: &Path,
    reference_bytes: &[u8],
    render_path: &Path,
    render_bytes: &[u8],
) -> Result<(Comparison, ComparisonImages)> {
    let reference = analysis::classify(reference_path, reference_bytes)?;
    let render = analysis::classify(render_path, render_bytes)?;

    if reference.width != render.width || reference.height != render.height {
        return Err(Error::CompareDimensionMismatch {
            reference_width: reference.width,
            reference_height: reference.height,
            render_width: render.width,
            render_height: render.height,
        });
    }
    let (width, height) = (reference.width, reference.height);
    let width_usize = width as usize;
    let total_pixels = u64::from(width) * u64::from(height);

    let canvas = Dimensions {
        width,
        height,
        aspect_ratio: f64::from(width) / f64::from(height),
    };

    // Appearance is a second look at the same bytes: `analyze` re-decodes, which
    // is cheap beside the perceptual pass and keeps `analysis` the only place a
    // region is defined.
    let (reference_analysis, reference_map) =
        analysis::analyze_with_regions(reference_path, reference_bytes)?;
    let (render_analysis, render_map) = analysis::analyze_with_regions(render_path, render_bytes)?;
    let appearance = appearance::compare_appearance(
        &reference_analysis,
        &reference_map,
        &render_analysis,
        &render_map,
    );

    let typography =
        typography::compare_typography(&reference_analysis.typography, &render_analysis.typography);

    let reference_background = background_of(&reference);
    let render_background = background_of(&render);

    let foreground = compare_masks(
        &reference.foreground_mask,
        &render.foreground_mask,
        total_pixels,
    );

    let reference_edges = edge_mask(&reference.pixels, width, height);
    let render_edges = edge_mask(&render.pixels, width, height);
    let edges = compare_masks(&reference_edges, &render_edges, total_pixels);

    let reference_bbox = analysis::whole_bounding_box(&reference.foreground_mask, width_usize);
    let render_bbox = analysis::whole_bounding_box(&render.foreground_mask, width_usize);
    let bounding_box = bounding_box_comparison(reference_bbox, render_bbox, width, height);

    let reference_centroid = analysis::whole_centroid(&reference.foreground_mask, width_usize);
    let render_centroid = analysis::whole_centroid(&render.foreground_mask, width_usize);
    let centroid = centroid_comparison(reference_centroid, render_centroid, width, height);

    let pixel_error = pixel_error(&reference.pixels, &render.pixels);

    let reference_image = RgbaImage::from_raw(width, height, reference.pixels.clone())
        .expect("classify() always returns exactly width*height*4 bytes");
    let render_image = RgbaImage::from_raw(width, height, render.pixels.clone())
        .expect("classify() always returns exactly width*height*4 bytes");
    let similarity = image_compare::rgba_hybrid_compare(&reference_image, &render_image)
        .map_err(|source| Error::PerceptualSimilarity { source })?;
    let perceptual_similarity = PerceptualSimilarity {
        algorithm: "rgba_hybrid",
        score: similarity.score,
    };

    let overlay = encode_png(&DynamicImage::ImageRgba8(build_overlay(
        &reference.foreground_mask,
        &render.foreground_mask,
        width,
        height,
    )));
    let difference = encode_png(&similarity.image.to_color_map());

    Ok((
        Comparison {
            canvas,
            reference_background,
            render_background,
            foreground,
            edges,
            bounding_box,
            centroid,
            pixel_error,
            perceptual_similarity,
            appearance,
            typography,
        },
        ComparisonImages {
            overlay,
            difference,
        },
    ))
}

/// Build an [`analysis::Background`] from what [`analysis::classify`]
/// already sampled — the same fields [`analysis::analyze`] reports, reused
/// rather than resampled.
fn background_of(classified: &Classified) -> Background {
    Background {
        transparent_fraction: classified.transparent_fraction,
        border_colour: classified
            .background_colour
            .map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")),
        border_uniformity: classified.border_uniformity,
    }
}

/// Overlap of two boolean masks of equal length.
fn compare_masks(reference: &[bool], render: &[bool], total_pixels: u64) -> MaskComparison {
    let mut intersection: u64 = 0;
    let mut union: u64 = 0;
    let mut reference_count: u64 = 0;
    let mut render_count: u64 = 0;

    for (&r, &g) in reference.iter().zip(render.iter()) {
        if r {
            reference_count += 1;
        }
        if g {
            render_count += 1;
        }
        if r && g {
            intersection += 1;
        }
        if r || g {
            union += 1;
        }
    }

    let reference_fraction = reference_count as f64 / total_pixels as f64;
    let render_fraction = render_count as f64 / total_pixels as f64;

    MaskComparison {
        reference_fraction,
        render_fraction,
        absolute_difference: (reference_fraction - render_fraction).abs(),
        intersection_over_union: if union == 0 {
            1.0
        } else {
            intersection as f64 / union as f64
        },
    }
}

fn bounding_box_comparison(
    reference: Option<BoundingBox>,
    render: Option<BoundingBox>,
    width: u32,
    height: u32,
) -> BoundingBoxComparison {
    let offset = reference.zip(render).map(|(r, g)| Offset {
        dx: (f64::from(g.x) + f64::from(g.width) / 2.0)
            - (f64::from(r.x) + f64::from(r.width) / 2.0),
        dy: (f64::from(g.y) + f64::from(g.height) / 2.0)
            - (f64::from(r.y) + f64::from(r.height) / 2.0),
    });
    let offset_fraction = offset.map(|offset| Offset {
        dx: offset.dx / f64::from(width),
        dy: offset.dy / f64::from(height),
    });
    let width_ratio = reference
        .zip(render)
        .map(|(r, g)| f64::from(g.width) / f64::from(r.width));
    let height_ratio = reference
        .zip(render)
        .map(|(r, g)| f64::from(g.height) / f64::from(r.height));

    BoundingBoxComparison {
        reference,
        render,
        offset,
        offset_fraction,
        width_ratio,
        height_ratio,
    }
}

fn centroid_comparison(
    reference: Option<Centroid>,
    render: Option<Centroid>,
    width: u32,
    height: u32,
) -> CentroidComparison {
    let offset = reference.zip(render).map(|(r, g)| Offset {
        dx: g.x - r.x,
        dy: g.y - r.y,
    });
    let offset_fraction = offset.map(|offset| Offset {
        dx: offset.dx / f64::from(width),
        dy: offset.dy / f64::from(height),
    });

    CentroidComparison {
        reference,
        render,
        offset,
        offset_fraction,
    }
}

/// Whole-canvas pixel error, over every channel of every pixel — background
/// included, since a colour shift in the background is still a real
/// difference between the two images.
fn pixel_error(reference: &[u8], render: &[u8]) -> PixelError {
    let mut sum_absolute = 0.0f64;
    let mut sum_squared = 0.0f64;
    let mut max_absolute = 0.0f64;
    let mut changed_pixels: u64 = 0;
    let mut pixel_count: u64 = 0;
    let mut channel_sum = [0.0f64; 4];

    for (reference_pixel, render_pixel) in reference
        .as_chunks::<4>()
        .0
        .iter()
        .zip(render.as_chunks::<4>().0.iter())
    {
        pixel_count += 1;
        let mut pixel_changed = false;
        for channel in 0..4 {
            let raw_difference = (i32::from(reference_pixel[channel])
                - i32::from(render_pixel[channel]))
            .unsigned_abs();
            if raw_difference >= u32::from(PIXEL_CHANGE_THRESHOLD) {
                pixel_changed = true;
            }
            let difference = f64::from(raw_difference) / 255.0;
            sum_absolute += difference;
            sum_squared += difference * difference;
            max_absolute = max_absolute.max(difference);
            channel_sum[channel] += difference;
        }
        if pixel_changed {
            changed_pixels += 1;
        }
    }

    let total_channel_samples = pixel_count as f64 * 4.0;
    let mean_squared_error = sum_squared / total_channel_samples;

    PixelError {
        changed_pixel_fraction: changed_pixels as f64 / pixel_count as f64,
        mean_absolute_error: sum_absolute / total_channel_samples,
        mean_squared_error,
        root_mean_squared_error: mean_squared_error.sqrt(),
        max_absolute_error: max_absolute,
        per_channel_mean_absolute_error: ChannelErrors {
            r: channel_sum[0] / pixel_count as f64,
            g: channel_sum[1] / pixel_count as f64,
            b: channel_sum[2] / pixel_count as f64,
            a: channel_sum[3] / pixel_count as f64,
        },
    }
}

/// A binary edge mask via a Sobel gradient magnitude over alpha-weighted
/// luminance, thresholded at [`EDGE_GRADIENT_THRESHOLD`]. Border pixels
/// clamp to the nearest real pixel rather than wrapping or zero-padding —
/// a logo's own edge is rarely at the canvas edge, but this keeps the
/// function total either way rather than special-casing the border.
fn edge_mask(pixels: &[u8], width: u32, height: u32) -> Vec<bool> {
    let width = width as usize;
    let height = height as usize;

    // Alpha-weighted so a transparent pixel contributes as black rather than
    // as whatever colour an encoder happened to leave behind it.
    let luminance: Vec<f32> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            let alpha = f32::from(pixel[3]) / 255.0;
            let luma = 0.299 * f32::from(pixel[0])
                + 0.587 * f32::from(pixel[1])
                + 0.114 * f32::from(pixel[2]);
            luma * alpha
        })
        .collect();

    let at = |x: i64, y: i64| -> f32 {
        let x = x.clamp(0, width as i64 - 1) as usize;
        let y = y.clamp(0, height as i64 - 1) as usize;
        luminance[y * width + x]
    };

    let mut mask = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let (xi, yi) = (x as i64, y as i64);
            let gx = at(xi - 1, yi - 1) + 2.0 * at(xi - 1, yi) + at(xi - 1, yi + 1)
                - at(xi + 1, yi - 1)
                - 2.0 * at(xi + 1, yi)
                - at(xi + 1, yi + 1);
            let gy = at(xi - 1, yi - 1) + 2.0 * at(xi, yi - 1) + at(xi + 1, yi - 1)
                - at(xi - 1, yi + 1)
                - 2.0 * at(xi, yi + 1)
                - at(xi + 1, yi + 1);
            let magnitude = gx.hypot(gy);
            mask[y * width + x] = magnitude >= EDGE_GRADIENT_THRESHOLD;
        }
    }
    mask
}

/// Colours are opaque and saturated on purpose — legible at the small sizes
/// a logo comparison usually happens at, not a subtle designer's overlay.
const OVERLAY_BOTH: [u8; 4] = [255, 255, 255, 255];
const OVERLAY_REFERENCE_ONLY: [u8; 4] = [255, 32, 96, 220];
const OVERLAY_RENDER_ONLY: [u8; 4] = [32, 200, 255, 220];

/// Paint where the two foreground masks agree and disagree, over a
/// transparent canvas.
fn build_overlay(
    reference_mask: &[bool],
    render_mask: &[bool],
    width: u32,
    height: u32,
) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for (index, pixel) in image.pixels_mut().enumerate() {
        *pixel = image::Rgba(match (reference_mask[index], render_mask[index]) {
            (true, true) => OVERLAY_BOTH,
            (true, false) => OVERLAY_REFERENCE_ONLY,
            (false, true) => OVERLAY_RENDER_ONLY,
            (false, false) => [0, 0, 0, 0],
        });
    }
    image
}

/// Encode an already-decoded or freshly-built image to PNG. Cannot fail for
/// real inputs — the buffer is already valid pixel data and the writer is an
/// in-memory `Vec`, so a genuine failure here would mean `image` itself is
/// broken, not that this call was given bad input.
fn encode_png(image: &DynamicImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding an already-decoded image to PNG never fails");
    bytes
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// A filled disc, `size` pixels square, centred on a transparent
    /// background, offset by `(offset_x, offset_y)` pixels and scaled by
    /// `scale` from the default radius — the one shape generator every test
    /// below configures rather than duplicating.
    fn disc_png(size: u32, offset_x: i32, offset_y: i32, scale: f64) -> Vec<u8> {
        let mut image = RgbaImage::new(size, size);
        let centre = f64::from(size) / 2.0;
        // A wider margin than `analysis`'s own disc fixture: this module's
        // tests shift the disc away from centre, and the shifted shape must
        // still clear the canvas border, or the border-colour sample gets
        // corrupted by the disc's own fill — see `classify`'s doc comment.
        let radius = (centre - 10.0) * scale;
        for y in 0..size {
            for x in 0..size {
                let dx = f64::from(x as i32 - offset_x) - centre;
                let dy = f64::from(y as i32 - offset_y) - centre;
                let inside = (dx * dx + dy * dy).sqrt() <= radius;
                image.put_pixel(
                    x,
                    y,
                    image::Rgba(if inside {
                        [20, 20, 20, 255]
                    } else {
                        [0, 0, 0, 0]
                    }),
                );
            }
        }
        encode(&image)
    }

    /// Two disjoint squares — a materially different shape from a disc, for
    /// the "different shapes" comparison case.
    fn two_squares_png(size: u32) -> Vec<u8> {
        let mut image = RgbaImage::new(size, size);
        let margin = 4;
        let small = size / 8;
        let big = size / 4;
        for y in 0..size {
            for x in 0..size {
                let in_small =
                    x >= margin && x < margin + small && y >= margin && y < margin + small;
                let in_big = x >= size - margin - big
                    && x < size - margin
                    && y >= size - margin - big
                    && y < size - margin;
                image.put_pixel(
                    x,
                    y,
                    image::Rgba(if in_small || in_big {
                        [10, 10, 10, 255]
                    } else {
                        [0, 0, 0, 0]
                    }),
                );
            }
        }
        encode(&image)
    }

    fn encode(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encoding a freshly-built test image never fails");
        bytes
    }

    fn path() -> std::path::PathBuf {
        std::path::PathBuf::from("<test>")
    }

    #[test]
    fn comparing_an_image_against_itself_reports_perfect_overlap_and_similarity() {
        let bytes = disc_png(64, 0, 0, 1.0);
        let (comparison, images) = compare(&path(), &bytes, &path(), &bytes).unwrap();

        assert_eq!(comparison.foreground.intersection_over_union, 1.0);
        assert_eq!(comparison.edges.intersection_over_union, 1.0);
        assert_eq!(comparison.pixel_error.changed_pixel_fraction, 0.0);
        assert_eq!(comparison.pixel_error.mean_absolute_error, 0.0);
        assert!(comparison.perceptual_similarity.score > 0.999);
        assert!(!images.overlay.is_empty());
        assert!(!images.difference.is_empty());
    }

    #[test]
    fn comparing_the_same_image_twice_produces_identical_output() {
        // Determinism: the property the whole comparison exists to have.
        let bytes = disc_png(64, 3, -2, 0.9);
        let (first, _) = compare(&path(), &bytes, &path(), &bytes).unwrap();
        let (second, _) = compare(&path(), &bytes, &path(), &bytes).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
    }

    #[test]
    fn comparing_a_translated_shape_reports_the_known_centroid_offset() {
        let reference = disc_png(64, 0, 0, 1.0);
        let render = disc_png(64, 6, -4, 1.0);
        let (comparison, _) = compare(&path(), &reference, &path(), &render).unwrap();

        let offset = comparison
            .centroid
            .offset
            .expect("both discs have foreground");
        // `disc_png(.., offset_x, offset_y, ..)` moves the drawn shape by
        // exactly `(offset_x, offset_y)` in image space, so the render's
        // centroid minus the reference's is that same vector.
        assert!((offset.dx - 6.0).abs() < 1.0, "{offset:?}");
        assert!((offset.dy - (-4.0)).abs() < 1.0, "{offset:?}");
        assert!(comparison.foreground.intersection_over_union < 1.0);
        assert!(comparison.foreground.intersection_over_union > 0.0);
    }

    #[test]
    fn comparing_a_resized_shape_reports_the_known_bounding_box_ratio() {
        let reference = disc_png(128, 0, 0, 1.0);
        let render = disc_png(128, 0, 0, 0.5);
        let (comparison, _) = compare(&path(), &reference, &path(), &render).unwrap();

        let width_ratio = comparison
            .bounding_box
            .width_ratio
            .expect("both discs have foreground");
        assert!((width_ratio - 0.5).abs() < 0.05, "{width_ratio}");
    }

    #[test]
    fn comparing_materially_different_shapes_reports_low_overlap_and_similarity() {
        let reference = disc_png(96, 0, 0, 1.0);
        let render = two_squares_png(96);
        let (comparison, _) = compare(&path(), &reference, &path(), &render).unwrap();

        assert!(comparison.foreground.intersection_over_union < 0.3);
        assert!(comparison.perceptual_similarity.score < 0.9);
        assert!(comparison.pixel_error.mean_absolute_error > 0.0);
    }

    #[test]
    fn a_dimension_mismatch_is_refused_rather_than_panicking() {
        let reference = disc_png(64, 0, 0, 1.0);
        let render = disc_png(32, 0, 0, 1.0);
        let error = compare(&path(), &reference, &path(), &render).unwrap_err();
        assert!(matches!(error, Error::CompareDimensionMismatch { .. }));
    }

    #[test]
    fn comparing_a_typical_logo_sized_reference_completes_quickly() {
        let reference = disc_png(512, 0, 0, 1.0);
        let render = disc_png(512, 4, -3, 0.95);

        let started = std::time::Instant::now();
        let (_, _) = compare(&path(), &reference, &path(), &render).unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "compare() took {:?} for a 512x512 pair",
            started.elapsed()
        );
    }
}
