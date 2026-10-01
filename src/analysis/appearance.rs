//! Measuring how a region is filled, rather than only which colour it is.
//!
//! [`super::analyze`] separates a raster into regions and dominant colours,
//! which is enough for artwork made of flat fills. It says nothing about a
//! region that fades from one colour to another, a fill that is translucent,
//! or a region that is really a line with a width — and an agent that takes a
//! gradient for a flat colour, or the reverse, draws the wrong thing.
//!
//! Everything here is measurement with an admitted margin of error:
//!
//! - A region is [`Fill::Flat`] when its colour barely varies, a gradient
//!   only when a model explains nearly all of the variation *and* the fitted
//!   profile is a smooth ramp, and [`Fill::Varied`] otherwise. `Varied` is
//!   the honest default: noise, a photograph, stripes, a hard step between
//!   two colours and a gradient too noisy to trust all land there, with both
//!   fits reported so "almost a gradient" is visible.
//! - Colour is measured in four channels, alpha included, so a fade of one
//!   colour into transparency is a gradient whose stops differ in opacity
//!   only.
//! - A stroke is a *candidate*: a thin, elongated, even-width region. A
//!   filled ring and a stroked circle are the same pixels, so the module
//!   reports the geometry and lets the reader decide what to draw.
//!
//! Only the pixels well inside a region are sampled (its anti-aliased rim is
//! eroded away), the sample is thinned by a fixed stride, ties are broken by
//! scan order, and directions use `sqrt` rather than trigonometry until the
//! final angle, which is rounded. The same bytes give the same report on any
//! machine.

use serde::Serialize;

use super::{Hole, Region, hex};

/// Alpha below this is not counted as part of any region (see
/// [`super::TRANSPARENT_ALPHA_THRESHOLD`]).
const TRANSLUCENT_FLOOR: u8 = super::TRANSPARENT_ALPHA_THRESHOLD;

/// Alpha at or above this counts as fully opaque. A little under 255 so that
/// the 8-bit rounding of a nominally opaque pixel is not called translucent.
pub(crate) const OPAQUE_FLOOR: u8 = 240;

/// The root-mean-square deviation of a region's channels, in levels of 0-255,
/// under which it is called flat. Roughly the noise floor of a lossy export:
/// a ramp has to span more than about 24 levels to clear it, and a gradient
/// subtler than that reads as flat, on purpose.
const FLAT_SPREAD: f64 = 6.0;

/// Fewest samples a gradient can be claimed from. Fewer than this cannot
/// distinguish a ramp from three unrelated pixels.
const MIN_FIT_SAMPLES: usize = 64;

/// Fewest pixels the eroded core keeps before erosion stops. A thin stroke
/// erodes to nothing, and its colour is better read from its rim than not
/// at all.
const MIN_CORE: usize = 16;

/// Most samples any fit reads. Thinning by a fixed stride keeps the cost
/// bounded on a large image without making the result depend on anything but
/// the pixels.
const MAX_SAMPLES: usize = 8192;

/// Most samples the search for a radial centre reads: it evaluates the model
/// a hundred-odd times, the final fit only once.
const SEARCH_SAMPLES: usize = 2048;

/// Bins in the profile a model is fitted to, at most. Fewer for a small
/// sample, so that a bin is never a handful of pixels.
const MAX_BINS: usize = 24;

/// Fewest bins a profile is fitted with.
const MIN_BINS: usize = 4;

/// Samples per bin, at least.
const MIN_PER_BIN: usize = 8;

/// Bins used while searching for a radial centre.
const SEARCH_BINS: usize = 16;

/// Shortest span, in pixels, along an axis or radius that can carry a
/// gradient. Shorter is a smudge.
const MIN_AXIS_EXTENT: f64 = 8.0;

/// Share of colour variance a model must explain to be believed (R²).
const ACCEPT_FIT: f64 = 0.9;

/// A single step between neighbouring bins carrying more than this share of
/// the profile's total change makes it a step, not a ramp.
const MAX_STEP_SHARE: f64 = 0.5;

/// A bin-to-bin change counts as a ramp change when it is at least this
/// fraction of the average change.
const RAMP_JUMP_FRACTION: f64 = 0.25;

/// Share of bin-to-bin changes that must be ramp changes. A profile that is
/// flat, jumps, and is flat again is a staircase, not a gradient.
const RAMP_JUMP_SHARE: f64 = 1.0 / 3.0;

/// A radial model must beat the linear one by this much of R² to be preferred
/// where both fit — a radial gradient centred at an edge mimics a linear ramp
/// on an elongated shape.
const RADIAL_MARGIN: f64 = 0.03;

/// Largest distance, over four channels, a profile point may sit from the
/// straight line between its neighbours before it earns a stop of its own.
const STOP_TOLERANCE: f64 = 6.0;

/// Most stops reported. A profile needing more is stripes, not a gradient.
const MAX_STOPS: usize = 6;

/// Fewest pixels a region needs to be considered for a stroke.
const STROKE_MIN_AREA: u64 = 12;

/// Length-to-width ratio from which a region reads as a line.
const STROKE_MIN_ELONGATION: f64 = 4.0;

/// Evenness of width (1 minus the variation of the ridge's distance values)
/// under which a region is a blob with a tail rather than a stroke.
const STROKE_MIN_UNIFORMITY: f64 = 0.6;

/// Elongation at which a stroke's confidence stops growing.
const STROKE_CONFIDENT_ELONGATION: f64 = 12.0;

/// Everything measured about how the image is filled.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Appearance {
    /// How much of the image is transparent, translucent or opaque.
    pub alpha: AlphaSummary,
    /// One entry per region in [`super::Analysis::regions`], in the same
    /// order.
    pub regions: Vec<RegionAppearance>,
}

/// How transparent the image is, measured on alpha alone — independent of
/// which colour the pixels are.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AlphaSummary {
    /// Fraction of every pixel that is transparent (alpha under 16).
    pub transparent_fraction: f64,
    /// Fraction of every pixel that is partly transparent (alpha from 16 to
    /// 239). This includes anti-aliased edges.
    pub translucent_fraction: f64,
    /// Fraction of every pixel that is opaque (alpha 240 or more).
    pub opaque_fraction: f64,
    /// Fraction of every pixel that is translucent *and* whose neighbours
    /// are all translucent too. An anti-aliased edge always touches an
    /// opaque or transparent pixel, so what remains is translucency that was
    /// drawn, not an artefact of an edge.
    pub interior_translucent_fraction: f64,
}

/// How one region is filled.
#[derive(Debug, Clone, Serialize)]
pub struct RegionAppearance {
    /// The [`Region::id`] this describes.
    pub region_id: usize,
    /// Whether the region is a flat colour, a gradient, or neither.
    pub fill: Fill,
    /// The region's alpha, apart from its colour, over its interior pixels.
    pub opacity: Opacity,
    /// Present when the region is thin, long and of even width.
    pub stroke: Option<Stroke>,
}

/// A point, in image pixel coordinates.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Point {
    /// Horizontal position.
    pub x: f64,
    /// Vertical position, growing downwards.
    pub y: f64,
}

/// How a region's colour is distributed.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fill {
    /// One colour, within the noise floor of the image.
    Flat {
        /// The mean colour, as `#rrggbb`.
        colour: String,
        /// Root-mean-square deviation of the channels, 0-255. Small but
        /// non-zero is a gradient too subtle to claim, or a lossy export.
        spread: f64,
    },
    /// Colour that ramps along one axis.
    LinearGradient {
        /// Where the ramp starts: the region's extreme along the axis.
        start: Point,
        /// Where the ramp ends.
        end: Point,
        /// The direction from `start` to `end`, in degrees, rounded to a
        /// tenth. `0` points right and `90` points down. The axis is always
        /// reported pointing right (or down when vertical), so a gradient
        /// and its reverse differ in their stops, not in their axis.
        angle_degrees: f64,
        /// Colours along the ramp, first to last.
        stops: Vec<GradientStop>,
        /// Share of the colour variance the profile explains, 0-1.
        fit: f64,
    },
    /// Colour that ramps with distance from a point.
    RadialGradient {
        /// The point the colour radiates from.
        centre: Point,
        /// Distance from `centre` to the region's farthest pixel: the extent
        /// of the region, which is where a gradient clipped by it stops.
        radius: f64,
        /// Colours from the centre outwards. Offsets are fractions of
        /// `radius`.
        stops: Vec<GradientStop>,
        /// Share of the colour variance the profile explains, 0-1.
        fit: f64,
    },
    /// Colour that varies, but not as a gradient this module will vouch for:
    /// several flat colours, texture, noise, stripes or a photograph.
    Varied {
        /// The mean colour, as `#rrggbb`.
        mean_colour: String,
        /// Root-mean-square deviation of the channels, 0-255.
        spread: f64,
        /// How well a linear ramp explains the variation, 0-1. High but
        /// still `varied` means the ramp was not smooth: a step or stripes.
        linear_fit: f64,
        /// How well a radial ramp explains the variation, 0-1.
        radial_fit: f64,
    },
}

/// One colour along a gradient.
#[derive(Debug, Clone, Serialize)]
pub struct GradientStop {
    /// Position along the gradient: 0 at its start (or centre), 1 at its end
    /// (or radius).
    pub offset: f64,
    /// The stop's colour, as `#rrggbb`, ignoring alpha.
    pub colour: String,
    /// The stop's opacity, 0-1, kept apart from the colour.
    pub opacity: f64,
}

/// A region's alpha, apart from its colour.
#[derive(Debug, Clone, Serialize)]
pub struct Opacity {
    /// Lowest opacity among the interior pixels, 0-1.
    pub min: f64,
    /// Mean opacity, 0-1.
    pub mean: f64,
    /// Highest opacity, 0-1.
    pub max: f64,
}

/// Geometry that looks like a stroke: a region much longer than it is wide.
#[derive(Debug, Clone, Serialize)]
pub struct Stroke {
    /// Typical width, in pixels, measured across the region's medial axis.
    pub width: f64,
    /// Approximate length along the stroke, in pixels: area over width.
    pub length: f64,
    /// `length / width`.
    pub elongation: f64,
    /// How even the width is along the stroke, 0-1.
    pub width_uniformity: f64,
    /// Whether the stroke encloses a hole, as a ring or a closed outline
    /// does.
    pub closed: bool,
    /// 0-1. Grows with elongation and with the evenness of the width. It is
    /// a measure of the geometry, not a claim that a stroke was drawn.
    pub confidence: f64,
}

/// One interior pixel of a region: where it is and its colour and alpha.
struct Sample {
    x: f64,
    y: f64,
    /// Red, green, blue and alpha, each 0-255.
    c: [f64; 4],
}

/// A region cut out of the image: its mask on a grid one pixel larger than
/// the bounding box on every side, so no neighbour lookup leaves the grid.
struct Local {
    width: usize,
    height: usize,
    /// Image x of local column 1.
    origin_x: usize,
    /// Image y of local row 1.
    origin_y: usize,
    mask: Vec<bool>,
}

/// Measure alpha and every reported region. `region_map` holds, for each
/// pixel, the id of the region it belongs to or [`super::NO_REGION`].
pub(super) fn measure(
    pixels: &[u8],
    width: usize,
    height: usize,
    region_map: &[u32],
    regions: &[Region],
    holes: &[Hole],
) -> Appearance {
    let regions = regions
        .iter()
        .map(|region| {
            let closed = holes.iter().any(|hole| hole.enclosed_by == Some(region.id));
            region_appearance(pixels, width, region_map, region, closed)
        })
        .collect();
    Appearance {
        alpha: alpha_summary(pixels, width, height),
        regions,
    }
}

/// Fractions of the image by alpha class, and the translucent interior.
fn alpha_summary(pixels: &[u8], width: usize, height: usize) -> AlphaSummary {
    let alpha = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3];
    let translucent = |a: u8| (TRANSLUCENT_FLOOR..OPAQUE_FLOOR).contains(&a);

    let (mut transparent, mut translucent_count, mut opaque, mut interior) =
        (0u64, 0u64, 0u64, 0u64);
    for y in 0..height {
        for x in 0..width {
            let a = alpha(x, y);
            if a < TRANSLUCENT_FLOOR {
                transparent += 1;
            } else if a >= OPAQUE_FLOOR {
                opaque += 1;
            } else {
                translucent_count += 1;
                // Only neighbours inside the image are asked: the canvas
                // edge is not evidence of an edge in the artwork.
                let surrounded = (x == 0 || translucent(alpha(x - 1, y)))
                    && (x + 1 == width || translucent(alpha(x + 1, y)))
                    && (y == 0 || translucent(alpha(x, y - 1)))
                    && (y + 1 == height || translucent(alpha(x, y + 1)));
                if surrounded {
                    interior += 1;
                }
            }
        }
    }
    let total = (width * height) as f64;
    AlphaSummary {
        transparent_fraction: transparent as f64 / total,
        translucent_fraction: translucent_count as f64 / total,
        opaque_fraction: opaque as f64 / total,
        interior_translucent_fraction: interior as f64 / total,
    }
}

/// Everything measured about one region.
fn region_appearance(
    pixels: &[u8],
    width: usize,
    region_map: &[u32],
    region: &Region,
    closed: bool,
) -> RegionAppearance {
    let local = local_mask(region, region_map, width);
    let samples = interior_samples(&local, pixels, width);

    let count = samples.len().max(1) as f64;
    let alphas = samples.iter().map(|s| s.c[3] / 255.0);
    let opacity = Opacity {
        min: alphas.clone().fold(f64::INFINITY, f64::min),
        mean: alphas.clone().sum::<f64>() / count,
        max: alphas.fold(0.0, f64::max),
    };

    RegionAppearance {
        region_id: region.id,
        fill: classify_fill(&samples, &footprint(&local), region),
        opacity,
        stroke: stroke_candidate(&local, region.area, closed),
    }
}

/// The region's mask on a padded local grid.
fn local_mask(region: &Region, region_map: &[u32], width: usize) -> Local {
    let bounds = region.bounding_box;
    let (bw, bh) = (bounds.width as usize, bounds.height as usize);
    let (origin_x, origin_y) = (bounds.x as usize, bounds.y as usize);
    let (local_width, local_height) = (bw + 2, bh + 2);
    let mut mask = vec![false; local_width * local_height];
    for ly in 0..bh {
        for lx in 0..bw {
            let index = (origin_y + ly) * width + origin_x + lx;
            mask[(ly + 1) * local_width + lx + 1] = region_map[index] == region.id as u32;
        }
    }
    Local {
        width: local_width,
        height: local_height,
        origin_x,
        origin_y,
        mask,
    }
}

/// Where every pixel of the region is, rim included.
fn footprint(local: &Local) -> Vec<(f64, f64)> {
    (0..local.mask.len())
        .filter(|&i| local.mask[i])
        .map(|i| {
            (
                (local.origin_x + i % local.width - 1) as f64,
                (local.origin_y + i / local.width - 1) as f64,
            )
        })
        .collect()
}

/// A pixel survives when it and its four neighbours are all in the mask.
fn erode(mask: &[bool], width: usize, height: usize) -> Vec<bool> {
    let mut out = vec![false; mask.len()];
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            out[i] = mask[i] && mask[i - 1] && mask[i + 1] && mask[i - width] && mask[i + width];
        }
    }
    out
}

/// The region's pixels away from its rim, thinned to at most
/// [`MAX_SAMPLES`] by a fixed stride, in scan order.
///
/// The rim of a region is where anti-aliasing blends it with its
/// surroundings, so its pixels measure the edge and not the fill. Erosion is
/// applied twice, since an anti-aliased edge can be two pixels deep, and
/// stops early rather than erode a thin region away.
fn interior_samples(local: &Local, pixels: &[u8], width: usize) -> Vec<Sample> {
    let mut core = local.mask.clone();
    for _ in 0..2 {
        let eroded = erode(&core, local.width, local.height);
        if eroded.iter().filter(|&&v| v).count() < MIN_CORE {
            break;
        }
        core = eroded;
    }

    let indices: Vec<usize> = (0..core.len()).filter(|&i| core[i]).collect();
    let stride = indices.len().div_ceil(MAX_SAMPLES).max(1);
    indices
        .into_iter()
        .step_by(stride)
        .map(|i| {
            let x = local.origin_x + i % local.width - 1;
            let y = local.origin_y + i / local.width - 1;
            let p = (y * width + x) * 4;
            Sample {
                x: x as f64,
                y: y as f64,
                c: [
                    f64::from(pixels[p]),
                    f64::from(pixels[p + 1]),
                    f64::from(pixels[p + 2]),
                    f64::from(pixels[p + 3]),
                ],
            }
        })
        .collect()
}

/// A colour as `#rrggbb`, clamped and rounded.
fn colour_hex(c: &[f64; 4]) -> String {
    let channel = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    hex([channel(c[0]), channel(c[1]), channel(c[2])])
}

/// An alpha channel as a 0-1 opacity.
fn opacity_of(c: &[f64; 4]) -> f64 {
    (c[3] / 255.0).clamp(0.0, 1.0)
}

/// Round to one decimal place.
fn tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Decide whether a region's samples are flat, a gradient, or varied.
fn classify_fill(samples: &[Sample], footprint: &[(f64, f64)], region: &Region) -> Fill {
    let count = samples.len().max(1) as f64;
    let mut mean = [0.0; 4];
    for s in samples {
        for (m, v) in mean.iter_mut().zip(s.c) {
            *m += v;
        }
    }
    for m in &mut mean {
        *m /= count;
    }
    let total_variance: f64 = samples
        .iter()
        .map(|s| {
            s.c.iter()
                .zip(mean)
                .map(|(v, m)| (v - m) * (v - m))
                .sum::<f64>()
        })
        .sum();
    let spread = (total_variance / (count * 4.0)).sqrt();

    if samples.len() < MIN_FIT_SAMPLES || spread <= FLAT_SPREAD {
        return Fill::Flat {
            colour: colour_hex(&mean),
            spread,
        };
    }

    let linear = fit_linear(samples, footprint, &mean, total_variance);
    let radial = fit_radial(samples, footprint, region, total_variance);
    let linear_fit = linear.as_ref().map_or(0.0, |m| m.profile.fit);
    let radial_fit = radial.as_ref().map_or(0.0, |m| m.profile.fit);

    let linear = linear.filter(|m| m.profile.stops.is_some());
    let radial = radial.filter(|m| m.profile.stops.is_some());
    let prefer_radial = match (&linear, &radial) {
        (Some(l), Some(r)) => r.profile.fit > l.profile.fit + RADIAL_MARGIN,
        (None, Some(_)) => true,
        _ => false,
    };

    if prefer_radial {
        let model = radial.expect("prefer_radial implies a radial model");
        Fill::RadialGradient {
            centre: model.centre,
            radius: model.radius,
            stops: model.profile.stops.expect("filtered to accepted models"),
            fit: model.profile.fit,
        }
    } else if let Some(model) = linear {
        Fill::LinearGradient {
            start: model.start,
            end: model.end,
            angle_degrees: model.angle_degrees,
            stops: model.profile.stops.expect("filtered to accepted models"),
            fit: model.profile.fit,
        }
    } else {
        Fill::Varied {
            mean_colour: colour_hex(&mean),
            spread,
            linear_fit,
            radial_fit,
        }
    }
}

/// A fitted one-dimensional colour profile.
struct Profile {
    /// Share of the colour variance the binned profile explains.
    fit: f64,
    /// The stops, only when the fit is good enough *and* the profile is a
    /// smooth ramp with few enough turns to be a gradient.
    stops: Option<Vec<GradientStop>>,
}

struct LinearModel {
    start: Point,
    end: Point,
    angle_degrees: f64,
    profile: Profile,
}

struct RadialModel {
    centre: Point,
    radius: f64,
    profile: Profile,
}

/// Fit a linear ramp: find the direction colour changes fastest in by least
/// squares, then read the colour profile along it.
fn fit_linear(
    samples: &[Sample],
    footprint: &[(f64, f64)],
    mean: &[f64; 4],
    total_variance: f64,
) -> Option<LinearModel> {
    let n = samples.len() as f64;
    let mx = samples.iter().map(|s| s.x).sum::<f64>() / n;
    let my = samples.iter().map(|s| s.y).sum::<f64>() / n;

    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    let (mut sxc, mut syc) = ([0.0; 4], [0.0; 4]);
    for s in samples {
        let (dx, dy) = (s.x - mx, s.y - my);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
        for k in 0..4 {
            let dc = s.c[k] - mean[k];
            sxc[k] += dx * dc;
            syc[k] += dy * dc;
        }
    }
    if sxx + syy <= 0.0 {
        return None;
    }

    // Per-channel gradient (how fast the channel changes in x and in y). A
    // region one pixel thick has no second dimension to solve for, so it
    // falls back to the one it has.
    let det = sxx * syy - sxy * sxy;
    let well_posed = det > 1e-9 * (sxx + syy) * (sxx + syy);
    let (mut a, mut b, mut d) = (0.0, 0.0, 0.0);
    for k in 0..4 {
        let (gx, gy) = if well_posed {
            (
                (syy * sxc[k] - sxy * syc[k]) / det,
                (sxx * syc[k] - sxy * sxc[k]) / det,
            )
        } else if sxx >= syy {
            (sxc[k] / sxx, 0.0)
        } else {
            (0.0, syc[k] / syy)
        };
        a += gx * gx;
        b += gx * gy;
        d += gy * gy;
    }

    // The dominant eigenvector of the summed outer products: the direction in
    // which all four channels together change fastest.
    let half_diff = (a - d) / 2.0;
    let lambda = (a + d) / 2.0 + (half_diff * half_diff + b * b).sqrt();
    let (mut vx, mut vy) = if b.abs() > 1e-12 {
        (b, lambda - a)
    } else if a >= d {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    let norm = (vx * vx + vy * vy).sqrt();
    if norm < 1e-12 {
        (vx, vy) = (1.0, 0.0);
    } else {
        (vx, vy) = (vx / norm, vy / norm);
    }
    // An axis has no direction until one is chosen; pointing right (or down
    // when vertical) makes a gradient and its reverse share an axis.
    if vx < -1e-9 || (vx.abs() <= 1e-9 && vy < 0.0) {
        (vx, vy) = (-vx, -vy);
    }

    let points: Vec<(f64, [f64; 4])> = samples
        .iter()
        .map(|s| ((s.x - mx) * vx + (s.y - my) * vy, s.c))
        .collect();
    // The extent comes from every pixel of the region, rim included, so the
    // axis runs edge to edge; the colours at its ends are then read off the
    // fitted line rather than from the eroded interior, which stops short.
    let along = |&(x, y): &(f64, f64)| (x - mx) * vx + (y - my) * vy;
    let t_min = footprint.iter().map(along).fold(f64::INFINITY, f64::min);
    let t_max = footprint
        .iter()
        .map(along)
        .fold(f64::NEG_INFINITY, f64::max);
    let profile = fit_profile(&points, total_variance, t_min, t_max, t_min, t_max - t_min);

    Some(LinearModel {
        start: Point {
            x: mx + vx * t_min,
            y: my + vy * t_min,
        },
        end: Point {
            x: mx + vx * t_max,
            y: my + vy * t_max,
        },
        angle_degrees: tenth(vy.atan2(vx).to_degrees()),
        profile,
    })
}

/// Squared-error cost of describing `radii` (with their colours) by one
/// colour per radius bin: how badly a centre explains the colours.
fn binned_residual(points: &[(f64, [f64; 4])], bins: usize) -> f64 {
    let lo = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let hi = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    if hi - lo <= 0.0 {
        return f64::MAX;
    }
    let bin_of = |u: f64| (((u - lo) / (hi - lo) * bins as f64) as usize).min(bins - 1);
    let mut sums = vec![[0.0; 4]; bins];
    let mut counts = vec![0.0_f64; bins];
    for (u, c) in points {
        let bin = bin_of(*u);
        counts[bin] += 1.0;
        for k in 0..4 {
            sums[bin][k] += c[k];
        }
    }
    let mut residual = 0.0;
    for (u, c) in points {
        let bin = bin_of(*u);
        for k in 0..4 {
            let e = c[k] - sums[bin][k] / counts[bin];
            residual += e * e;
        }
    }
    residual
}

/// Fit a radial ramp: search the region's bounding box for the centre whose
/// distance profile explains the colours best, then read the profile there.
fn fit_radial(
    samples: &[Sample],
    footprint: &[(f64, f64)],
    region: &Region,
    total_variance: f64,
) -> Option<RadialModel> {
    let stride = samples.len().div_ceil(SEARCH_SAMPLES).max(1);
    let search: Vec<&Sample> = samples.iter().step_by(stride).collect();
    let cost = |cx: f64, cy: f64| {
        let points: Vec<(f64, [f64; 4])> = search
            .iter()
            .map(|s| (((s.x - cx).powi(2) + (s.y - cy).powi(2)).sqrt(), s.c))
            .collect();
        binned_residual(&points, SEARCH_BINS)
    };

    let bounds = region.bounding_box;
    let (x0, y0) = (f64::from(bounds.x), f64::from(bounds.y));
    let (span_x, span_y) = (
        f64::from(bounds.width.saturating_sub(1)),
        f64::from(bounds.height.saturating_sub(1)),
    );

    // A coarse grid over the box, then finer neighbourhoods around the best
    // so far. Strict `<` keeps the first of equal candidates in scan order.
    let mut best = (x0 + span_x / 2.0, y0 + span_y / 2.0);
    let mut best_cost = f64::MAX;
    for gy in 0..=8 {
        for gx in 0..=8 {
            let (cx, cy) = (
                x0 + span_x * f64::from(gx) / 8.0,
                y0 + span_y * f64::from(gy) / 8.0,
            );
            let c = cost(cx, cy);
            if c < best_cost {
                (best, best_cost) = ((cx, cy), c);
            }
        }
    }
    let (mut step_x, mut step_y) = (span_x / 8.0, span_y / 8.0);
    for _ in 0..4 {
        let centre = best;
        for gy in -2..=2 {
            for gx in -2..=2 {
                let cx = (centre.0 + step_x * f64::from(gx)).clamp(x0, x0 + span_x);
                let cy = (centre.1 + step_y * f64::from(gy)).clamp(y0, y0 + span_y);
                let c = cost(cx, cy);
                if c < best_cost {
                    (best, best_cost) = ((cx, cy), c);
                }
            }
        }
        step_x /= 2.0;
        step_y /= 2.0;
    }

    let points: Vec<(f64, [f64; 4])> = samples
        .iter()
        .map(|s| {
            (
                ((s.x - best.0).powi(2) + (s.y - best.1).powi(2)).sqrt(),
                s.c,
            )
        })
        .collect();
    // As for a linear ramp, the extent is the whole region's, rim included.
    let away = |&(x, y): &(f64, f64)| ((x - best.0).powi(2) + (y - best.1).powi(2)).sqrt();
    let r_min = footprint.iter().map(away).fold(f64::INFINITY, f64::min);
    let r_max = footprint.iter().map(away).fold(f64::NEG_INFINITY, f64::max);
    if r_max <= 0.0 {
        return None;
    }
    let profile = fit_profile(&points, total_variance, r_min, r_max, 0.0, r_max);
    Some(RadialModel {
        centre: Point {
            x: best.0,
            y: best.1,
        },
        radius: r_max,
        profile,
    })
}

/// Bin colours by `u` (position along an axis, or distance from a centre),
/// score how much of the variance the bins explain, and, if that is nearly
/// all of it and the profile is a smooth ramp, turn it into stops.
///
/// `offset = (u - origin) / scale`, so a linear profile is measured from its
/// start and a radial one from its centre.
fn fit_profile(
    points: &[(f64, [f64; 4])],
    total_variance: f64,
    u_min: f64,
    u_max: f64,
    origin: f64,
    scale: f64,
) -> Profile {
    let rejected = |fit: f64| Profile { fit, stops: None };
    let extent = u_max - u_min;
    let bins = (points.len() / MIN_PER_BIN).min(MAX_BINS);
    if extent < MIN_AXIS_EXTENT || bins < MIN_BINS || total_variance <= 0.0 {
        return rejected(0.0);
    }

    let bin_of = |u: f64| (((u - u_min) / extent * bins as f64) as usize).min(bins - 1);
    let mut sums = vec![[0.0; 4]; bins];
    let mut position = vec![0.0; bins];
    let mut counts = vec![0.0_f64; bins];
    for (u, c) in points {
        let bin = bin_of(*u);
        counts[bin] += 1.0;
        position[bin] += u;
        for k in 0..4 {
            sums[bin][k] += c[k];
        }
    }
    let means: Vec<[f64; 4]> = (0..bins)
        .map(|b| sums[b].map(|v| v / counts[b].max(1.0)))
        .collect();

    let mut residual = 0.0;
    for (u, c) in points {
        let bin = bin_of(*u);
        for k in 0..4 {
            let e = c[k] - means[bin][k];
            residual += e * e;
        }
    }
    let fit = (1.0 - residual / total_variance).clamp(0.0, 1.0);
    if fit < ACCEPT_FIT {
        return rejected(fit);
    }

    // The profile: one point per non-empty bin, in order along `u`.
    let profile: Vec<(f64, [f64; 4])> = (0..bins)
        .filter(|&b| counts[b] > 0.0)
        .map(|b| (position[b] / counts[b], means[b]))
        .collect();
    if !is_smooth_ramp(&profile) {
        return rejected(fit);
    }

    let mut keep = vec![false; profile.len()];
    keep[0] = true;
    *keep.last_mut().expect("a profile has points") = true;
    simplify(&profile, 0, profile.len() - 1, &mut keep);
    let kept = merge_corner_stops(&profile, (0..profile.len()).filter(|&i| keep[i]).collect());
    if kept.len() > MAX_STOPS {
        return rejected(fit);
    }

    let offset_of = |u: f64| (u - origin) / scale;
    let stop = |offset: f64, c: &[f64; 4]| GradientStop {
        offset,
        colour: colour_hex(c),
        opacity: opacity_of(c),
    };
    let stops = if kept.len() == 2 {
        // Two stops: read the ends off the least-squares line through all of
        // the samples, which sits at the true ends rather than half a bin
        // in from them.
        let (start, end) = line_ends(points, u_min, u_max);
        vec![stop(offset_of(u_min), &start), stop(offset_of(u_max), &end)]
    } else {
        kept.iter()
            .enumerate()
            .map(|(n, &i)| {
                let u = if n == 0 {
                    u_min
                } else if n == kept.len() - 1 {
                    u_max
                } else {
                    profile[i].0
                };
                stop(offset_of(u), &profile[i].1)
            })
            .collect()
    };
    Profile {
        fit,
        stops: Some(stops),
    }
}

/// The least-squares line through `points`, evaluated at `u_min` and `u_max`.
fn line_ends(points: &[(f64, [f64; 4])], u_min: f64, u_max: f64) -> ([f64; 4], [f64; 4]) {
    let n = points.len() as f64;
    let mean_u = points.iter().map(|p| p.0).sum::<f64>() / n;
    let mut mean_c = [0.0; 4];
    for (_, c) in points {
        for k in 0..4 {
            mean_c[k] += c[k] / n;
        }
    }
    let var_u: f64 = points.iter().map(|p| (p.0 - mean_u).powi(2)).sum();
    let mut slope = [0.0; 4];
    if var_u > 0.0 {
        for (u, c) in points {
            for k in 0..4 {
                slope[k] += (u - mean_u) * (c[k] - mean_c[k]) / var_u;
            }
        }
    }
    let at = |u: f64| {
        let mut out = [0.0; 4];
        for k in 0..4 {
            out[k] = mean_c[k] + slope[k] * (u - mean_u);
        }
        out
    };
    (at(u_min), at(u_max))
}

/// Distance between two colours, over four channels.
fn colour_distance(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}

/// Whether a binned profile changes steadily rather than in a step, a
/// staircase or a flicker. A profile of two flat colours has a perfect fit
/// and is still not a gradient.
fn is_smooth_ramp(profile: &[(f64, [f64; 4])]) -> bool {
    let jumps: Vec<f64> = profile
        .windows(2)
        .map(|w| colour_distance(&w[0].1, &w[1].1))
        .collect();
    if jumps.len() < 2 {
        return false;
    }
    let total: f64 = jumps.iter().sum();
    if total <= 0.0 {
        return false;
    }
    let largest = jumps.iter().copied().fold(0.0, f64::max);
    let mean = total / jumps.len() as f64;
    let ramping = jumps
        .iter()
        .filter(|&&j| j >= mean * RAMP_JUMP_FRACTION)
        .count();
    largest < total * MAX_STEP_SHARE && ramping as f64 >= jumps.len() as f64 * RAMP_JUMP_SHARE
}

/// Douglas-Peucker over a colour profile: keep the point farthest from the
/// straight line between the kept ends, while it is farther than
/// [`STOP_TOLERANCE`]. Ties keep the earliest point.
fn simplify(profile: &[(f64, [f64; 4])], lo: usize, hi: usize, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let (u0, c0) = profile[lo];
    let (u1, c1) = profile[hi];
    let (mut worst, mut worst_index) = (0.0, lo);
    for (i, (u, c)) in profile.iter().enumerate().take(hi).skip(lo + 1) {
        let t = if u1 > u0 { (u - u0) / (u1 - u0) } else { 0.0 };
        let expected = [0, 1, 2, 3].map(|k| c0[k] + (c1[k] - c0[k]) * t);
        let distance = colour_distance(c, &expected);
        if distance > worst {
            (worst, worst_index) = (distance, i);
        }
    }
    if worst > STOP_TOLERANCE {
        keep[worst_index] = true;
        simplify(profile, lo, worst_index, keep);
        simplify(profile, worst_index, hi, keep);
    }
}

/// Collapse two interior stops that sit within a bin of each other into one.
///
/// A bin that straddles a corner of the profile averages the two slopes, so
/// its mean sags off the corner and the bin beside it is kept as well. Both
/// describe the one corner; the one farther from the line between the
/// profile's ends is nearer to it.
fn merge_corner_stops(profile: &[(f64, [f64; 4])], kept: Vec<usize>) -> Vec<usize> {
    let last = profile.len() - 1;
    let deviation = |i: usize| {
        let (u0, c0) = profile[0];
        let (u1, c1) = profile[last];
        let t = if u1 > u0 {
            (profile[i].0 - u0) / (u1 - u0)
        } else {
            0.0
        };
        let expected = [0, 1, 2, 3].map(|k| c0[k] + (c1[k] - c0[k]) * t);
        colour_distance(&profile[i].1, &expected)
    };
    let mut merged: Vec<usize> = Vec::new();
    for i in kept {
        match merged.last().copied() {
            Some(previous) if previous != 0 && i != last && i - previous <= 2 => {
                if deviation(i) > deviation(previous) {
                    *merged.last_mut().expect("just matched a last element") = i;
                }
            }
            _ => merged.push(i),
        }
    }
    merged
}

/// 3-4 chamfer distance from each pixel of the mask to the nearest pixel
/// outside it, in thirds of a pixel.
fn chamfer(mask: &[bool], width: usize, height: usize) -> Vec<u32> {
    let mut dt: Vec<u32> = mask
        .iter()
        .map(|&m| if m { u32::MAX / 2 } else { 0 })
        .collect();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if !mask[i] {
                continue;
            }
            dt[i] = dt[i]
                .min(dt[i - 1] + 3)
                .min(dt[i - width] + 3)
                .min(dt[i - width - 1] + 4)
                .min(dt[i - width + 1] + 4);
        }
    }
    for y in (1..height - 1).rev() {
        for x in (1..width - 1).rev() {
            let i = y * width + x;
            if !mask[i] {
                continue;
            }
            dt[i] = dt[i]
                .min(dt[i + 1] + 3)
                .min(dt[i + width] + 3)
                .min(dt[i + width + 1] + 4)
                .min(dt[i + width - 1] + 4);
        }
    }
    dt
}

/// Report a stroke when the region is much longer than it is wide, and about
/// as wide everywhere.
///
/// Width comes from the medial axis: the ridge of the distance transform is
/// as far from either edge as it gets, so twice its distance to the nearest
/// outside pixel, less one, is the width of the run of pixels across it.
fn stroke_candidate(local: &Local, area: u64, closed: bool) -> Option<Stroke> {
    if area < STROKE_MIN_AREA {
        return None;
    }
    let (w, h) = (local.width, local.height);
    let dt = chamfer(&local.mask, w, h);

    let mut ridge = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if !local.mask[i] {
                continue;
            }
            let is_ridge = [
                i - 1,
                i + 1,
                i - w,
                i + w,
                i - w - 1,
                i - w + 1,
                i + w - 1,
                i + w + 1,
            ]
            .iter()
            .all(|&n| dt[n] <= dt[i]);
            if is_ridge {
                ridge.push(dt[i]);
            }
        }
    }
    if ridge.is_empty() {
        return None;
    }
    ridge.sort_unstable();
    let median = ridge[ridge.len() / 2];
    let mean = ridge.iter().map(|&v| f64::from(v)).sum::<f64>() / ridge.len() as f64;
    let variance = ridge
        .iter()
        .map(|&v| (f64::from(v) - mean).powi(2))
        .sum::<f64>()
        / ridge.len() as f64;
    let width_uniformity = (1.0 - variance.sqrt() / mean).clamp(0.0, 1.0);

    let width = (2.0 * f64::from(median) / 3.0 - 1.0).max(1.0);
    let length = area as f64 / width;
    let elongation = length / width;
    if elongation < STROKE_MIN_ELONGATION || width_uniformity < STROKE_MIN_UNIFORMITY {
        return None;
    }
    Some(Stroke {
        width,
        length,
        elongation,
        width_uniformity,
        closed,
        confidence: width_uniformity * (elongation / STROKE_CONFIDENT_ELONGATION).min(1.0),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use image::{Rgba, RgbaImage};

    use super::*;
    use crate::analysis::{Analysis, analyze};

    const SIZE: u32 = 96;
    /// Margin of transparent canvas around every shape: the background is
    /// sampled from the border, so artwork touching it would be swallowed.
    const MARGIN: u32 = 12;

    fn encode(img: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding a freshly-built test image never fails");
        bytes
    }

    /// A transparent canvas with the square `MARGIN..SIZE-MARGIN` painted by
    /// `paint(x, y)`, which is given coordinates relative to that square.
    fn square(paint: impl Fn(u32, u32) -> [u8; 4]) -> Analysis {
        let mut img = RgbaImage::new(SIZE, SIZE);
        for y in MARGIN..SIZE - MARGIN {
            for x in MARGIN..SIZE - MARGIN {
                img.put_pixel(x, y, Rgba(paint(x - MARGIN, y - MARGIN)));
            }
        }
        analyze(Path::new("test.png"), &encode(&img)).unwrap()
    }

    /// The side of the painted square.
    const SIDE: u32 = SIZE - 2 * MARGIN;

    fn lerp(a: [u8; 4], b: [u8; 4], num: u32, den: u32) -> [u8; 4] {
        let mix = |i: usize| {
            ((u32::from(a[i]) * (den - num) + u32::from(b[i]) * num + den / 2) / den) as u8
        };
        [mix(0), mix(1), mix(2), mix(3)]
    }

    const RED: [u8; 4] = [200, 30, 30, 255];
    const BLUE: [u8; 4] = [30, 40, 210, 255];
    const GREEN: [u8; 4] = [30, 170, 60, 255];

    fn fill_of(analysis: &Analysis) -> &Fill {
        assert!(!analysis.appearance.regions.is_empty(), "no region found");
        &analysis.appearance.regions[0].fill
    }

    fn colour_within(hex: &str, expected: [u8; 4], tolerance: i32) -> bool {
        (0..3).all(|i| {
            let got = i32::from_str_radix(&hex[1 + 2 * i..3 + 2 * i], 16).unwrap();
            (got - i32::from(expected[i])).abs() <= tolerance
        })
    }

    #[test]
    fn a_solid_rectangle_is_flat() {
        let analysis = square(|_, _| RED);
        match fill_of(&analysis) {
            Fill::Flat { colour, spread } => {
                assert_eq!(colour, "#c81e1e");
                assert_eq!(*spread, 0.0);
            }
            other => panic!("expected flat, got {other:?}"),
        }
    }

    #[test]
    fn an_anti_aliased_disc_is_flat_because_its_rim_is_not_sampled() {
        let mut img = RgbaImage::new(SIZE, SIZE);
        let centre = f64::from(SIZE) / 2.0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let d = ((f64::from(x) + 0.5 - centre).powi(2)
                    + (f64::from(y) + 0.5 - centre).powi(2))
                .sqrt();
                // One pixel of coverage ramp at the rim, like any renderer.
                let coverage = (30.5 - d).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    img.put_pixel(x, y, Rgba([20, 20, 20, (coverage * 255.0) as u8]));
                }
            }
        }
        let analysis = analyze(Path::new("disc.png"), &encode(&img)).unwrap();
        assert!(
            matches!(fill_of(&analysis), Fill::Flat { .. }),
            "{:?}",
            fill_of(&analysis)
        );
    }

    #[test]
    fn a_horizontal_ramp_is_a_linear_gradient_along_x() {
        let analysis = square(|x, _| lerp(RED, BLUE, x, SIDE - 1));
        let Fill::LinearGradient {
            start,
            end,
            angle_degrees,
            stops,
            fit,
        } = fill_of(&analysis)
        else {
            panic!("expected a linear gradient, got {:?}", fill_of(&analysis));
        };
        assert!(angle_degrees.abs() <= 2.0, "{angle_degrees}");
        assert!(start.x < end.x);
        assert!(*fit > 0.99, "{fit}");
        assert_eq!(stops.len(), 2, "{stops:?}");
        assert!(colour_within(&stops[0].colour, RED, 8), "{stops:?}");
        assert!(colour_within(&stops[1].colour, BLUE, 8), "{stops:?}");
        assert_eq!((stops[0].offset, stops[1].offset), (0.0, 1.0));
    }

    #[test]
    fn a_vertical_ramp_points_down() {
        let analysis = square(|_, y| lerp(RED, BLUE, y, SIDE - 1));
        let Fill::LinearGradient { angle_degrees, .. } = fill_of(&analysis) else {
            panic!("expected a linear gradient, got {:?}", fill_of(&analysis));
        };
        assert!((angle_degrees - 90.0).abs() <= 2.0, "{angle_degrees}");
    }

    #[test]
    fn a_diagonal_ramp_points_at_forty_five_degrees() {
        let analysis = square(|x, y| lerp(RED, BLUE, x + y, 2 * (SIDE - 1)));
        let Fill::LinearGradient { angle_degrees, .. } = fill_of(&analysis) else {
            panic!("expected a linear gradient, got {:?}", fill_of(&analysis));
        };
        assert!((angle_degrees - 45.0).abs() <= 2.0, "{angle_degrees}");
    }

    #[test]
    fn the_reverse_ramp_shares_the_axis_and_swaps_the_stops() {
        let forward = square(|x, _| lerp(RED, BLUE, x, SIDE - 1));
        let reverse = square(|x, _| lerp(BLUE, RED, x, SIDE - 1));
        let (
            Fill::LinearGradient {
                angle_degrees: a,
                stops: forward,
                ..
            },
            Fill::LinearGradient {
                angle_degrees: b,
                stops: reverse,
                ..
            },
        ) = (fill_of(&forward), fill_of(&reverse))
        else {
            panic!("both should be linear gradients");
        };
        assert_eq!(a, b);
        assert!(colour_within(&reverse[0].colour, BLUE, 8), "{reverse:?}");
        assert!(colour_within(&forward[0].colour, RED, 8), "{forward:?}");
    }

    #[test]
    fn a_three_colour_ramp_reports_the_middle_stop() {
        let half = (SIDE - 1) / 2;
        let analysis = square(|x, _| {
            if x <= half {
                lerp(RED, GREEN, x, half)
            } else {
                lerp(GREEN, BLUE, x - half, SIDE - 1 - half)
            }
        });
        let Fill::LinearGradient { stops, .. } = fill_of(&analysis) else {
            panic!("expected a linear gradient, got {:?}", fill_of(&analysis));
        };
        assert_eq!(stops.len(), 3, "{stops:?}");
        assert!((stops[1].offset - 0.5).abs() < 0.1, "{stops:?}");
        assert!(colour_within(&stops[1].colour, GREEN, 16), "{stops:?}");
    }

    #[test]
    fn a_fade_to_transparency_keeps_its_colour_and_ramps_only_the_opacity() {
        let analysis = square(|x, _| {
            let alpha = 20 + (x * 235 / (SIDE - 1)) as u8;
            [200, 30, 30, alpha]
        });
        let Fill::LinearGradient { stops, .. } = fill_of(&analysis) else {
            panic!("expected a linear gradient, got {:?}", fill_of(&analysis));
        };
        assert!(stops.first().unwrap().opacity < 0.2, "{stops:?}");
        assert!(stops.last().unwrap().opacity > 0.95, "{stops:?}");
        for stop in stops {
            assert!(
                colour_within(&stop.colour, [200, 30, 30, 0], 2),
                "{stops:?}"
            );
        }
    }

    #[test]
    fn a_radial_ramp_reports_its_centre_and_radius() {
        let mid = f64::from(SIDE) / 2.0 - 0.5;
        let analysis = square(|x, y| {
            let d = ((f64::from(x) - mid).powi(2) + (f64::from(y) - mid).powi(2)).sqrt();
            lerp(RED, BLUE, (d * 4.0).round().min(4.0 * 42.0) as u32, 4 * 42)
        });
        let Fill::RadialGradient {
            centre,
            radius,
            stops,
            fit,
        } = fill_of(&analysis)
        else {
            panic!("expected a radial gradient, got {:?}", fill_of(&analysis));
        };
        let expected = f64::from(MARGIN) + mid;
        assert!(
            (centre.x - expected).abs() <= 2.0 && (centre.y - expected).abs() <= 2.0,
            "{centre:?}"
        );
        // The square's farthest corner, not where the ramp stops.
        assert!(*radius > 45.0 && *radius < 55.0, "{radius}");
        assert!(*fit > 0.95, "{fit}");
        assert!(stops.len() >= 2);
    }

    #[test]
    fn an_off_centre_radial_ramp_finds_the_off_centre_point() {
        let (cx, cy) = (20.0, 25.0);
        let analysis = square(|x, y| {
            let d = ((f64::from(x) - cx).powi(2) + (f64::from(y) - cy).powi(2)).sqrt();
            lerp(RED, BLUE, (d * 4.0).round().min(4.0 * 90.0) as u32, 4 * 90)
        });
        let Fill::RadialGradient { centre, .. } = fill_of(&analysis) else {
            panic!("expected a radial gradient, got {:?}", fill_of(&analysis));
        };
        assert!(
            (centre.x - (cx + f64::from(MARGIN))).abs() <= 3.0,
            "{centre:?}"
        );
        assert!(
            (centre.y - (cy + f64::from(MARGIN))).abs() <= 3.0,
            "{centre:?}"
        );
    }

    /// A deterministic pseudo-random byte in `0..=255` per pixel.
    fn noise(x: u32, y: u32) -> u32 {
        let mut h = x
            .wrapping_mul(374_761_393)
            .wrapping_add(y.wrapping_mul(668_265_263));
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        (h ^ (h >> 16)) >> 8 & 0xff
    }

    fn assert_not_a_gradient(analysis: &Analysis, what: &str) {
        let fill = fill_of(analysis);
        assert!(
            matches!(fill, Fill::Flat { .. } | Fill::Varied { .. }),
            "{what} was reported as a gradient: {fill:?}"
        );
    }

    #[test]
    fn noise_over_a_flat_fill_is_not_a_gradient() {
        let analysis = square(|x, y| {
            let n = noise(x, y);
            [(n as u8) / 2 + 60, (n as u8) / 3 + 60, 100, 255]
        });
        assert_not_a_gradient(&analysis, "noise");
        assert!(matches!(fill_of(&analysis), Fill::Varied { .. }));
    }

    #[test]
    fn noise_that_swamps_a_ramp_is_not_a_gradient() {
        let analysis = square(|x, y| {
            let base = lerp(RED, BLUE, x, SIDE - 1);
            let jitter = |v: u8| (i32::from(v) + noise(x, y) as i32 - 128).clamp(0, 255) as u8;
            [jitter(base[0]), jitter(base[1]), jitter(base[2]), 255]
        });
        assert_not_a_gradient(&analysis, "a ramp under heavy noise");
    }

    #[test]
    fn a_hard_two_colour_step_is_not_a_gradient() {
        let analysis = square(|x, _| if x < SIDE / 2 { RED } else { BLUE });
        assert_not_a_gradient(&analysis, "a step");
        let Fill::Varied { linear_fit, .. } = fill_of(&analysis) else {
            panic!("expected varied, got {:?}", fill_of(&analysis));
        };
        // The fit is near perfect and the fill is still not a gradient:
        // that is the point of the smoothness test.
        assert!(*linear_fit > 0.9, "{linear_fit}");
    }

    #[test]
    fn stripes_are_not_a_gradient() {
        let analysis = square(|x, _| match (x / 6) % 3 {
            0 => RED,
            1 => GREEN,
            _ => BLUE,
        });
        assert_not_a_gradient(&analysis, "stripes");
    }

    #[test]
    fn a_checkerboard_is_not_a_gradient() {
        let analysis = square(|x, y| if (x / 8 + y / 8) % 2 == 0 { RED } else { BLUE });
        assert_not_a_gradient(&analysis, "a checkerboard");
    }

    #[test]
    fn three_flat_bands_are_not_a_gradient() {
        let third = SIDE / 3;
        let analysis = square(|x, _| match x / third {
            0 => RED,
            1 => GREEN,
            _ => BLUE,
        });
        assert_not_a_gradient(&analysis, "a staircase");
    }

    #[test]
    fn a_ramp_too_subtle_to_tell_from_noise_is_flat() {
        let analysis = square(|x, _| lerp([100, 100, 100, 255], [108, 108, 108, 255], x, SIDE - 1));
        assert!(
            matches!(fill_of(&analysis), Fill::Flat { .. }),
            "{:?}",
            fill_of(&analysis)
        );
    }

    #[test]
    fn a_uniformly_translucent_fill_is_flat_with_a_half_opacity() {
        let analysis = square(|_, _| [200, 30, 30, 128]);
        assert!(matches!(fill_of(&analysis), Fill::Flat { .. }));
        let opacity = &analysis.appearance.regions[0].opacity;
        assert!((opacity.mean - 128.0 / 255.0).abs() < 1e-9, "{opacity:?}");
        assert!(analysis.appearance.alpha.interior_translucent_fraction > 0.3);
        assert_eq!(analysis.appearance.alpha.opaque_fraction, 0.0);
    }

    #[test]
    fn an_opaque_shape_has_no_interior_translucency() {
        let analysis = square(|_, _| RED);
        let alpha = &analysis.appearance.alpha;
        assert_eq!(alpha.interior_translucent_fraction, 0.0);
        assert_eq!(alpha.translucent_fraction, 0.0);
        assert!(alpha.transparent_fraction > 0.0 && alpha.opaque_fraction > 0.0);
        assert!((alpha.transparent_fraction + alpha.opaque_fraction - 1.0).abs() < 1e-9);
    }

    #[test]
    fn an_anti_aliased_edge_is_not_interior_translucency() {
        let mut img = RgbaImage::new(SIZE, SIZE);
        let centre = f64::from(SIZE) / 2.0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let d = ((f64::from(x) + 0.5 - centre).powi(2)
                    + (f64::from(y) + 0.5 - centre).powi(2))
                .sqrt();
                let coverage = (30.5 - d).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    img.put_pixel(x, y, Rgba([20, 20, 20, (coverage * 255.0) as u8]));
                }
            }
        }
        let analysis = analyze(Path::new("disc.png"), &encode(&img)).unwrap();
        assert!(analysis.appearance.alpha.translucent_fraction > 0.0);
        assert_eq!(analysis.appearance.alpha.interior_translucent_fraction, 0.0);
    }

    #[test]
    fn a_translucent_fill_weighs_less_than_an_opaque_one_among_dominant_colours() {
        let analysis = square(|x, _| if x < SIDE / 2 { RED } else { [30, 40, 210, 32] });
        let share = |prefix: &str| {
            analysis
                .dominant_colours
                .iter()
                .find(|c| c.colour.starts_with(prefix))
                .map_or(0.0, |c| c.fraction)
        };
        // Equal areas; the faint half must not count as much as the solid.
        assert!(
            share("#d0") > share("#10") * 3.0,
            "{:?}",
            analysis.dominant_colours
        );
    }

    /// A stroked shape on a transparent canvas.
    fn shape(inside: impl Fn(f64, f64) -> bool) -> Analysis {
        let mut img = RgbaImage::new(SIZE, SIZE);
        for y in 0..SIZE {
            for x in 0..SIZE {
                if inside(f64::from(x) - 47.5, f64::from(y) - 47.5) {
                    img.put_pixel(x, y, Rgba([20, 20, 20, 255]));
                }
            }
        }
        analyze(Path::new("shape.png"), &encode(&img)).unwrap()
    }

    #[test]
    fn a_thin_ring_is_a_closed_stroke_with_its_width() {
        let analysis = shape(|dx, dy| {
            let d = (dx * dx + dy * dy).sqrt();
            (18.0..22.0).contains(&d)
        });
        let stroke = analysis.appearance.regions[0]
            .stroke
            .as_ref()
            .expect("a ring is stroke-like");
        assert!((stroke.width - 4.0).abs() <= 1.5, "{stroke:?}");
        assert!(stroke.closed);
        assert!(stroke.elongation > 10.0, "{stroke:?}");
    }

    #[test]
    fn a_thin_bar_is_an_open_stroke() {
        let analysis = shape(|dx, dy| dx.abs() < 20.0 && dy.abs() < 1.6);
        let stroke = analysis.appearance.regions[0]
            .stroke
            .as_ref()
            .expect("a bar is stroke-like");
        assert!((stroke.width - 3.0).abs() <= 1.0, "{stroke:?}");
        assert!(!stroke.closed);
    }

    #[test]
    fn a_disc_a_square_and_a_block_are_not_strokes() {
        let disc = shape(|dx, dy| dx * dx + dy * dy < 20.0 * 20.0);
        let square = shape(|dx, dy| dx.abs() < 20.0 && dy.abs() < 20.0);
        let block = shape(|dx, dy| dx.abs() < 20.0 && dy.abs() < 10.0);
        for (name, analysis) in [("disc", disc), ("square", square), ("block", block)] {
            assert!(
                analysis.appearance.regions[0].stroke.is_none(),
                "{name}: {:?}",
                analysis.appearance.regions[0].stroke
            );
        }
    }

    #[test]
    fn analysing_a_gradient_twice_gives_identical_output() {
        let bytes = {
            let mut img = RgbaImage::new(SIZE, SIZE);
            for y in MARGIN..SIZE - MARGIN {
                for x in MARGIN..SIZE - MARGIN {
                    img.put_pixel(x, y, Rgba(lerp(RED, BLUE, x - MARGIN, SIDE - 1)));
                }
            }
            encode(&img)
        };
        let first = analyze(Path::new("g.png"), &bytes).unwrap();
        let second = analyze(Path::new("g.png"), &bytes).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
    }
}
