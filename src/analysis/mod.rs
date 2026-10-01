//! Measuring a raster reference into objective facts, rather than describing
//! it.
//!
//! An LLM looking at a reference image can name what it sees ("a hexagon, a
//! shackle, a keyhole") but has no mechanism for reading an actual pixel
//! count, bounding box or centroid off it — those numbers get guessed toward
//! "plausible for this kind of icon", the same gap [`crate::vectorize`]
//! closes for curves. This module closes it for layout and colour: [`analyze`]
//! turns a reference's pixels into dimensions, a background/foreground split,
//! dominant colours, connected regions, holes and symmetry scores, and
//! [`Appearance`] says how each region is filled. See ADR 018 for the
//! decision and why this is a new module rather than an extension of
//! `vectorize`, and ADR 025 for appearance.
//!
//! Self-contained and deterministic: bytes in, a typed [`Analysis`] out, no
//! knowledge of `Project`, tools or MCP — mirroring [`crate::vectorize`]'s own
//! isolation. Given the same bytes this produces the same output on any
//! machine, proven by `analysing_the_same_image_twice_produces_identical_output`
//! below.
//!
//! Everything here is measurement, not interpretation: a [`Region`] is a
//! bounding box, an area and a centroid, never a name. Only one background
//! colour is ever separated from everything else, sampled from the image's
//! own border — the same single-colour assumption `vectorize`'s binary
//! tracer makes, and just as unreliable for a photograph or a busy,
//! multi-region background.

use std::collections::BTreeMap;
use std::path::Path;

use image::GenericImageView;
use serde::Serialize;

use crate::error::{Error, Result};

mod appearance;
pub(crate) use appearance::OPAQUE_FLOOR;

pub use appearance::{
    AlphaSummary, Appearance, Fill, GradientStop, Opacity, Point, RegionAppearance, Stroke,
};

/// What a pixel's entry in the region map holds when it belongs to no region.
pub(crate) const NO_REGION: u32 = u32::MAX;

/// A pixel whose alpha is below this is still counted as fully transparent — real anti-aliased edges rarely land
/// exactly at 0.
const TRANSPARENT_ALPHA_THRESHOLD: u8 = 16;

/// Squared RGB distance under which a pixel is considered close enough to
/// the sampled border colour to count as background. Roughly a 32-per-channel
/// tolerance (`32 * 32 * 3`), wide enough for anti-aliasing without
/// swallowing a genuinely different foreground colour.
const BACKGROUND_DISTANCE_THRESHOLD: u32 = 3072;

/// Width of one bucket in the dominant-colour histogram, per channel.
/// Coarse on purpose: a handful of candidate colours is useful context, a
/// histogram with one entry per distinct pixel value is not.
const COLOUR_BUCKET_SIZE: u32 = 32;

/// How many dominant colours are reported, largest share first.
const MAX_DOMINANT_COLOURS: usize = 8;

/// How many regions are reported, largest area first. `region_count` still
/// reports the true total, so a caller can tell a fragmented result (many
/// regions, most of them tiny) from a clean one even when the list itself is
/// capped.
const MAX_REGIONS: usize = 32;

/// How many holes are reported, largest area first. See [`MAX_REGIONS`].
const MAX_HOLES: usize = 32;

/// Everything [`analyze`] measures about a reference image.
#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    /// The image's size and aspect ratio.
    pub dimensions: Dimensions,
    /// What the image's border says about its background.
    pub background: Background,
    /// The most common colours among opaque pixels, largest share first.
    pub dominant_colours: Vec<DominantColour>,
    /// Everything that is not background, taken together.
    pub foreground: Foreground,
    /// How many separate foreground regions were found, before `regions` is
    /// capped at [`MAX_REGIONS`].
    pub region_count: usize,
    /// Foreground regions, largest area first, capped at [`MAX_REGIONS`].
    pub regions: Vec<Region>,
    /// How many holes were found, before `holes` is capped at [`MAX_HOLES`].
    pub hole_count: usize,
    /// Background pixels fully enclosed by foreground, largest area first,
    /// capped at [`MAX_HOLES`].
    pub holes: Vec<Hole>,
    /// Mirror-symmetry scores of the foreground shape.
    pub symmetry: Symmetry,
    /// How the image is filled: its alpha, apart from colour, and for each
    /// of `regions` whether it is a flat colour, a gradient or neither, and
    /// whether it is shaped like a stroke.
    pub appearance: Appearance,
}

/// An image's size, in pixels.
#[derive(Debug, Clone, Serialize)]
pub struct Dimensions {
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
    /// `width / height`.
    pub aspect_ratio: f64,
}

/// What the image's border says about its background.
#[derive(Debug, Clone, Serialize)]
pub struct Background {
    /// Fraction of every pixel in the image below
    /// [`TRANSPARENT_ALPHA_THRESHOLD`].
    pub transparent_fraction: f64,
    /// The most common colour among opaque border pixels, as `#rrggbb`.
    /// `None` when every border pixel is transparent.
    pub border_colour: Option<String>,
    /// Fraction of border pixels that exactly match `border_colour` — a
    /// transparent border pixel never matches, so a border that is part
    /// transparent and part opaque scores below 1.0. `1.0` when
    /// `border_colour` is `None`, since every border pixel then agrees on
    /// being transparent.
    pub border_uniformity: f64,
}

/// One candidate colour in the image, and how much of it there is.
#[derive(Debug, Clone, Serialize)]
pub struct DominantColour {
    /// The bucket's representative colour, as `#rrggbb`.
    pub colour: String,
    /// Share of the image's opaque weight falling in this colour's bucket. A
    /// pixel weighs its alpha, so a half-transparent pixel counts half and a
    /// translucent fill or a soft edge is not mistaken for a full-strength
    /// colour.
    pub fraction: f64,
}

/// A rectangle, in image pixel coordinates. `x`/`y` is the top-left corner.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BoundingBox {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
}

/// A shape's mean pixel position, in image pixel coordinates.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Centroid {
    /// Mean `x`.
    pub x: f64,
    /// Mean `y`.
    pub y: f64,
}

/// Everything that is not background, taken together, without regard to how
/// many separate regions it forms.
#[derive(Debug, Clone, Serialize)]
pub struct Foreground {
    /// Share of every pixel classified as foreground.
    pub pixel_fraction: f64,
    /// The bounding box of every foreground pixel together. `None` when
    /// nothing was classified as foreground.
    pub bounding_box: Option<BoundingBox>,
}

/// One 4-connected foreground region.
#[derive(Debug, Clone, Serialize)]
pub struct Region {
    /// Assigned after sorting by area, largest first — stable regardless of
    /// which pixel a flood fill happened to start from, and stable across
    /// truncation to [`MAX_REGIONS`], so a [`Hole::enclosed_by`] naming a
    /// truncated region's id is still meaningful.
    pub id: usize,
    /// The region's bounding box.
    pub bounding_box: BoundingBox,
    /// Pixel count.
    pub area: u64,
    /// Mean pixel position.
    pub centroid: Centroid,
}

/// A background region fully enclosed by foreground — the image never
/// reaches the canvas border without crossing a foreground pixel first.
#[derive(Debug, Clone, Serialize)]
pub struct Hole {
    /// The hole's bounding box.
    pub bounding_box: BoundingBox,
    /// Pixel count.
    pub area: u64,
    /// Mean pixel position.
    pub centroid: Centroid,
    /// The enclosing region's id, when exactly one region's bounding box
    /// contains the hole's. `None` when no region does, or more than one
    /// does and which one is the "real" enclosure is ambiguous. May name a
    /// region that was truncated out of `regions` — see [`Region::id`].
    pub enclosed_by: Option<usize>,
}

/// Mirror-symmetry of the foreground shape, as an intersection-over-union of
/// the foreground mask with its own mirror image — `1.0` is a perfect
/// mirror, `0.0` shares no foreground pixel with its reflection at all.
#[derive(Debug, Clone, Serialize)]
pub struct Symmetry {
    /// Left-right mirror score.
    pub horizontal: f64,
    /// Top-bottom mirror score.
    pub vertical: f64,
}

/// Everything [`analyze`] and [`crate::compare`] both need from a decoded
/// image before their reports diverge: the raw pixels and a foreground/
/// background split. [`classify`] is that shared first step — see ADR 018
/// for why `#5`'s comparison tool reuses this rather than reimplementing its
/// own background/foreground split.
pub(crate) struct Classified {
    /// Image width, in pixels.
    pub width: u32,
    /// Image height, in pixels.
    pub height: u32,
    /// The decoded pixels, RGBA8, row-major — the same buffer
    /// `image::DynamicImage::to_rgba8().into_raw()` produces.
    pub pixels: Vec<u8>,
    /// The colour sampled from the border, if any border pixel was opaque.
    /// `None` when the whole border is transparent.
    pub background_colour: Option<[u8; 3]>,
    /// Fraction of every pixel below [`TRANSPARENT_ALPHA_THRESHOLD`].
    pub transparent_fraction: f64,
    /// Fraction of border pixels agreeing with `background_colour` — see
    /// [`Background::border_uniformity`], which this becomes verbatim.
    pub border_uniformity: f64,
    /// Whether each pixel (row-major, same order as `pixels`) was classified
    /// as foreground.
    pub foreground_mask: Vec<bool>,
}

/// Decode `bytes` (PNG/JPEG/GIF/WEBP/BMP — the formats
/// [`crate::tools`]'s `get_reference_image` already recognises) and classify
/// every pixel as background or foreground. `path` is only used to name the
/// reference in an error; it is never read.
///
/// The background is sampled from the image's own border, so artwork drawn
/// all the way to the canvas edge — with no margin at all — corrupts the
/// very sample everything else here is measured against. A reference
/// exported the way a logo normally is, with a little padding, is unaffected.
///
/// # Errors
///
/// Returns [`Error::AnalysisDecode`] if `bytes` is not a recognisable image.
pub(crate) fn classify(path: &Path, bytes: &[u8]) -> Result<Classified> {
    let decoded = image::load_from_memory(bytes).map_err(|source| Error::AnalysisDecode {
        path: path.to_path_buf(),
        source,
    })?;

    let (width, height) = decoded.dimensions();
    let pixels = decoded.to_rgba8().into_raw();
    let total_pixels = (width as usize) * (height as usize);

    // --- background: sampled from the border, never the whole canvas ---
    let mut transparent_count: u64 = 0;
    for pixel in pixels.as_chunks::<4>().0 {
        if pixel[3] < TRANSPARENT_ALPHA_THRESHOLD {
            transparent_count += 1;
        }
    }
    let transparent_fraction = transparent_count as f64 / total_pixels as f64;

    let border_positions = border_positions(width, height);
    let mut border_colour_counts: BTreeMap<[u8; 3], u64> = BTreeMap::new();
    for &(x, y) in &border_positions {
        let pixel = pixel_at(&pixels, width, x, y);
        if pixel[3] >= TRANSPARENT_ALPHA_THRESHOLD {
            *border_colour_counts
                .entry([pixel[0], pixel[1], pixel[2]])
                .or_insert(0) += 1;
        }
    }
    let border_colour = most_common(&border_colour_counts);
    let border_uniformity = match border_colour {
        Some((colour, _)) => {
            let matching = *border_colour_counts.get(&colour).unwrap_or(&0);
            matching as f64 / border_positions.len() as f64
        }
        // Every border pixel is transparent, so every one of them agrees.
        None => 1.0,
    };

    // --- classify every pixel as background or foreground ---
    let foreground_mask: Vec<bool> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            let rgb = [pixel[0], pixel[1], pixel[2]];
            let is_background = pixel[3] < TRANSPARENT_ALPHA_THRESHOLD
                || border_colour.is_some_and(|(colour, _)| {
                    squared_distance(colour, rgb) <= BACKGROUND_DISTANCE_THRESHOLD
                });
            !is_background
        })
        .collect();

    Ok(Classified {
        width,
        height,
        pixels,
        background_colour: border_colour.map(|(colour, _)| colour),
        transparent_fraction,
        border_uniformity,
        foreground_mask,
    })
}

/// Measure a reference's pixels into a structured, typed report — dimensions,
/// a background/foreground split, dominant colours, connected regions, holes
/// and symmetry scores. See the module doc comment and ADR 018 for what each
/// field means and why. `path` is only used to name the reference in an
/// error; it is never read.
///
/// # Errors
///
/// Returns [`Error::AnalysisDecode`] if `bytes` is not a recognisable image.
pub fn analyze(path: &Path, bytes: &[u8]) -> Result<Analysis> {
    analyze_with_regions(path, bytes).map(|(analysis, _)| analysis)
}

/// [`analyze`], and also which region each pixel belongs to: row-major, one
/// entry per pixel, the [`Region::id`] it was reported under or
/// [`NO_REGION`]. Ids beyond [`MAX_REGIONS`] are present in the map though not
/// in [`Analysis::regions`].
///
/// This is what lets [`crate::vectorize`] act on a region's *pixels* using
/// the same measurement an agent reads, rather than re-deriving regions
/// with a second, possibly different, segmentation.
pub(crate) fn analyze_with_regions(path: &Path, bytes: &[u8]) -> Result<(Analysis, Vec<u32>)> {
    let Classified {
        width,
        height,
        pixels,
        background_colour,
        transparent_fraction,
        border_uniformity,
        foreground_mask,
    } = classify(path, bytes)?;

    let width_usize = width as usize;
    let height_usize = height as usize;
    let total_pixels = width_usize * height_usize;

    let dimensions = Dimensions {
        width,
        height,
        aspect_ratio: f64::from(width) / f64::from(height),
    };

    let background = Background {
        transparent_fraction,
        border_colour: background_colour.map(hex),
        border_uniformity,
    };

    let foreground_count = foreground_mask.iter().filter(|&&value| value).count() as u64;

    // --- dominant colours, among non-transparent pixels, weighed by alpha ---
    // A pixel counts as much as it is opaque: counted whole, a translucent
    // fill or a soft edge would read as a full-strength colour. For an
    // opaque image every weight is 255 and this is a plain share of pixels.
    let mut colour_weights: BTreeMap<[u8; 3], u64> = BTreeMap::new();
    let mut total_weight: u64 = 0;
    for pixel in pixels.as_chunks::<4>().0 {
        if pixel[3] >= TRANSPARENT_ALPHA_THRESHOLD {
            let weight = u64::from(pixel[3]);
            total_weight += weight;
            let bucket = [
                bucket_channel(pixel[0]),
                bucket_channel(pixel[1]),
                bucket_channel(pixel[2]),
            ];
            *colour_weights.entry(bucket).or_insert(0) += weight;
        }
    }
    let dominant_colours = top_by_count(colour_weights)
        .into_iter()
        .take(MAX_DOMINANT_COLOURS)
        .map(|(colour, weight)| DominantColour {
            colour: hex(colour),
            fraction: if total_weight == 0 {
                0.0
            } else {
                weight as f64 / total_weight as f64
            },
        })
        .collect();

    // --- foreground, taken as a whole ---
    let foreground_bbox = whole_bounding_box(&foreground_mask, width_usize);
    let foreground = Foreground {
        pixel_fraction: foreground_count as f64 / total_pixels as f64,
        bounding_box: foreground_bbox,
    };

    // --- regions: 4-connected components of the foreground mask ---
    let (raw_regions, labels) = label_components(&foreground_mask, width_usize, height_usize);
    let region_count = raw_regions.len();
    let mut region_stats: Vec<_> = raw_regions
        .iter()
        .enumerate()
        .map(|(index, component)| (bbox(component), component.area, centroid(component), index))
        .collect();
    // Sorted by shape (area, then top-left corner), never by which pixel a
    // flood fill happened to start from, so `id` is stable regardless of scan
    // order and survives truncation to `MAX_REGIONS` unchanged.
    region_stats.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0.y.cmp(&b.0.y))
            .then(a.0.x.cmp(&b.0.x))
    });
    // Which region each flood-fill component became once sorted, so a pixel's
    // label can be turned into the id a caller sees.
    let mut region_of_component = vec![NO_REGION; raw_regions.len()];
    for (id, stats) in region_stats.iter().enumerate() {
        region_of_component[stats.3] = id as u32;
    }
    let region_map: Vec<u32> = labels
        .iter()
        .map(|&label| {
            if label == NO_REGION {
                NO_REGION
            } else {
                region_of_component[label as usize]
            }
        })
        .collect();
    let all_regions: Vec<Region> = region_stats
        .into_iter()
        .enumerate()
        .map(|(id, (bounding_box, area, centroid, _))| Region {
            id,
            bounding_box,
            area,
            centroid,
        })
        .collect();
    let regions: Vec<Region> = all_regions.iter().take(MAX_REGIONS).cloned().collect();

    // --- holes: background components that never touch the canvas border ---
    let background_mask: Vec<bool> = foreground_mask.iter().map(|value| !value).collect();
    let (raw_background, _) = label_components(&background_mask, width_usize, height_usize);
    let mut hole_stats: Vec<_> = raw_background
        .iter()
        .filter(|component| !component.touches_border)
        .map(|component| (bbox(component), component.area, centroid(component)))
        .collect();
    let hole_count = hole_stats.len();
    hole_stats.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0.y.cmp(&b.0.y))
            .then(a.0.x.cmp(&b.0.x))
    });
    let holes: Vec<Hole> = hole_stats
        .into_iter()
        .take(MAX_HOLES)
        .map(|(bounding_box, area, centroid)| {
            // Checked against every region, not only the truncated list, so
            // an enclosure by a small region that lost its place in
            // `regions` is still reported rather than silently dropped.
            let enclosing: Vec<_> = all_regions
                .iter()
                .filter(|region| contains(&region.bounding_box, &bounding_box))
                .collect();
            let enclosed_by = match enclosing.as_slice() {
                [only] => Some(only.id),
                _ => None,
            };
            Hole {
                bounding_box,
                area,
                centroid,
                enclosed_by,
            }
        })
        .collect();

    // --- symmetry, of the foreground mask against its own mirror ---
    let symmetry = Symmetry {
        horizontal: mirror_iou(
            &foreground_mask,
            width_usize,
            height_usize,
            Axis::Horizontal,
        ),
        vertical: mirror_iou(&foreground_mask, width_usize, height_usize, Axis::Vertical),
    };

    // --- appearance: how each reported region is filled ---
    let appearance = appearance::measure(
        &pixels,
        width_usize,
        height_usize,
        &region_map,
        &regions,
        &holes,
    );

    Ok((
        Analysis {
            dimensions,
            background,
            dominant_colours,
            foreground,
            region_count,
            regions,
            hole_count,
            holes,
            symmetry,
            appearance,
        },
        region_map,
    ))
}

/// Every pixel position on the image's border, corners counted once.
/// Degenerates gracefully for a one-pixel-wide or one-pixel-tall image.
fn border_positions(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut positions = Vec::new();
    for x in 0..width {
        positions.push((x, 0));
        if height > 1 {
            positions.push((x, height - 1));
        }
    }
    for y in 1..height.saturating_sub(1) {
        positions.push((0, y));
        if width > 1 {
            positions.push((width - 1, y));
        }
    }
    positions
}

/// Read one RGBA8 pixel out of a row-major buffer.
fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * width + x) * 4) as usize;
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

/// The most common colour in `counts`, with its count. Ties keep the
/// smallest RGB triple: `BTreeMap` iterates ascending, and only a *strictly*
/// larger count replaces the current best, so the winner never depends on
/// hashing or insertion order.
fn most_common(counts: &BTreeMap<[u8; 3], u64>) -> Option<([u8; 3], u64)> {
    let mut best: Option<([u8; 3], u64)> = None;
    for (&colour, &count) in counts {
        if best.is_none_or(|(_, best_count)| count > best_count) {
            best = Some((colour, count));
        }
    }
    best
}

/// Every entry of `counts`, sorted by count descending. `BTreeMap::into_iter`
/// yields ascending keys, and `sort_by` is stable, so entries tied on count
/// keep that ascending order rather than whatever order a hash map would
/// have produced.
fn top_by_count(counts: BTreeMap<[u8; 3], u64>) -> Vec<([u8; 3], u64)> {
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    entries
}

/// Round `value` to the centre of its [`COLOUR_BUCKET_SIZE`]-wide bucket.
fn bucket_channel(value: u8) -> u8 {
    let half = COLOUR_BUCKET_SIZE / 2;
    let bucket = (u32::from(value) / COLOUR_BUCKET_SIZE) * COLOUR_BUCKET_SIZE + half;
    bucket.min(255) as u8
}

/// Squared Euclidean distance between two RGB colours.
fn squared_distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let diff = i32::from(x) - i32::from(y);
            (diff * diff) as u32
        })
        .sum()
}

/// A colour as CSS hex, lowercase, always six digits.
fn hex(colour: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2])
}

/// The bounding box of every `true` pixel in `mask` together, ignoring which
/// component each belongs to. `None` when `mask` holds no `true` pixel.
///
/// `pub(crate)` rather than private: [`crate::compare`] measures a
/// foreground mask's bounding box the same way, against a second image, and
/// reuses this rather than reimplementing it.
pub(crate) fn whole_bounding_box(mask: &[bool], width: usize) -> Option<BoundingBox> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for (index, &value) in mask.iter().enumerate() {
        if !value {
            continue;
        }
        any = true;
        let x = (index % width) as u32;
        let y = (index / width) as u32;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    any.then(|| BoundingBox {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

/// The mean pixel position of every `true` pixel in `mask` together. `None`
/// when `mask` holds no `true` pixel — the same condition under which
/// [`whole_bounding_box`] returns `None`.
///
/// `pub(crate)` for the same reason as [`whole_bounding_box`]: shared with
/// [`crate::compare`].
pub(crate) fn whole_centroid(mask: &[bool], width: usize) -> Option<Centroid> {
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut count: u64 = 0;
    for (index, &value) in mask.iter().enumerate() {
        if !value {
            continue;
        }
        count += 1;
        sum_x += (index % width) as f64;
        sum_y += (index / width) as f64;
    }
    (count > 0).then(|| Centroid {
        x: sum_x / count as f64,
        y: sum_y / count as f64,
    })
}

/// A single flood-filled component's raw statistics, before it is turned
/// into a [`Region`] or a [`Hole`] and sorted.
struct RawComponent {
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    area: u64,
    sum_x: f64,
    sum_y: f64,
    /// Whether any of this component's pixels sit on the canvas edge —
    /// what distinguishes the "outer" background, which always touches the
    /// border, from a hole, which never does.
    touches_border: bool,
}

/// Label every 4-connected `true` region of `mask` (row-major, `width` x
/// `height`).
///
/// Scan order only decides which pixel starts each flood fill; every
/// reported statistic is a property of the region's shape, not of scan
/// order, so relabelling by area afterwards never changes what is measured —
/// only the order and the `id` it is measured under.
///
/// Also returns each pixel's component, as an index into the list —
/// [`NO_REGION`] for a pixel outside the mask — so a caller can get from a
/// region back to its pixels.
fn label_components(mask: &[bool], width: usize, height: usize) -> (Vec<RawComponent>, Vec<u32>) {
    let mut visited = vec![false; mask.len()];
    let mut labels = vec![NO_REGION; mask.len()];
    let mut components = Vec::new();

    for start in 0..mask.len() {
        if !mask[start] || visited[start] {
            continue;
        }

        let label = components.len() as u32;
        let mut stack = vec![start];
        visited[start] = true;
        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut area: u64 = 0;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut touches_border = false;

        while let Some(index) = stack.pop() {
            labels[index] = label;
            let x = (index % width) as u32;
            let y = (index / width) as u32;

            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            area += 1;
            sum_x += f64::from(x);
            sum_y += f64::from(y);
            if x == 0 || y == 0 || x as usize == width - 1 || y as usize == height - 1 {
                touches_border = true;
            }

            let neighbours = [
                (x > 0).then(|| index - 1),
                (x as usize + 1 < width).then(|| index + 1),
                (y > 0).then(|| index - width),
                (y as usize + 1 < height).then(|| index + width),
            ];
            for neighbour in neighbours.into_iter().flatten() {
                if mask[neighbour] && !visited[neighbour] {
                    visited[neighbour] = true;
                    stack.push(neighbour);
                }
            }
        }

        components.push(RawComponent {
            min_x,
            min_y,
            max_x,
            max_y,
            area,
            sum_x,
            sum_y,
            touches_border,
        });
    }

    (components, labels)
}

fn bbox(component: &RawComponent) -> BoundingBox {
    BoundingBox {
        x: component.min_x,
        y: component.min_y,
        width: component.max_x - component.min_x + 1,
        height: component.max_y - component.min_y + 1,
    }
}

fn centroid(component: &RawComponent) -> Centroid {
    Centroid {
        x: component.sum_x / component.area as f64,
        y: component.sum_y / component.area as f64,
    }
}

/// Whether `outer` fully contains `inner` — what distinguishes "this hole
/// sits inside this region" from merely overlapping it.
fn contains(outer: &BoundingBox, inner: &BoundingBox) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.x + outer.width >= inner.x + inner.width
        && outer.y + outer.height >= inner.y + inner.height
}

/// Which way `mask` is mirrored before comparing it with itself.
enum Axis {
    /// Left-right: column `x` compares against column `width - 1 - x`.
    Horizontal,
    /// Top-bottom: row `y` compares against row `height - 1 - y`.
    Vertical,
}

/// Intersection-over-union of `mask` with its own mirror image along `axis`.
/// `1.0` when there is nothing to disagree about — no foreground pixel at
/// all — rather than the `0 / 0` that would otherwise produce.
fn mirror_iou(mask: &[bool], width: usize, height: usize, axis: Axis) -> f64 {
    let mut intersection: u64 = 0;
    let mut union: u64 = 0;

    for y in 0..height {
        for x in 0..width {
            let (mirror_y, mirror_x) = match axis {
                Axis::Horizontal => (y, width - 1 - x),
                Axis::Vertical => (height - 1 - y, x),
            };
            let a = mask[y * width + x];
            let b = mask[mirror_y * width + mirror_x];
            if a && b {
                intersection += 1;
            }
            if a || b {
                union += 1;
            }
        }
    }

    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filled disc, `size` pixels square, centred, on a transparent
    /// background.
    fn disc_png(size: u32) -> Vec<u8> {
        shape_png(size, |dx, dy, outer, inner| {
            let d = (dx * dx + dy * dy).sqrt();
            d <= outer && d >= inner
        })
    }

    /// A ring: a filled circle with a smaller circle cut from its centre, on
    /// a transparent background — one region, one hole.
    fn ring_png(size: u32) -> Vec<u8> {
        shape_png(size, |dx, dy, outer, _inner| {
            let d = (dx * dx + dy * dy).sqrt();
            let inner = outer / 2.5;
            d <= outer && d >= inner
        })
    }

    fn shape_png(size: u32, inside: impl Fn(f64, f64, f64, f64) -> bool) -> Vec<u8> {
        let mut img = image::RgbaImage::new(size, size);
        let center = f64::from(size) / 2.0;
        let outer = center - 4.0;
        for y in 0..size {
            for x in 0..size {
                let dx = f64::from(x) - center;
                let dy = f64::from(y) - center;
                let pixel = if inside(dx, dy, outer, 0.0) {
                    image::Rgba([20, 20, 20, 255])
                } else {
                    image::Rgba([0, 0, 0, 0])
                };
                img.put_pixel(x, y, pixel);
            }
        }
        encode(&img)
    }

    /// Two disjoint, differently-sized filled squares on a transparent
    /// background — a small one in the top-left area, a bigger one in the
    /// bottom-right, with a gap between them and a margin from the canvas
    /// edge (background sampling reads the border, so artwork touching it
    /// would corrupt the very sample it is measured against — the same
    /// margin any exported logo already has in practice).
    fn two_squares_png(size: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::new(size, size);
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
                let pixel = if in_small || in_big {
                    image::Rgba([10, 10, 10, 255])
                } else {
                    image::Rgba([0, 0, 0, 0])
                };
                img.put_pixel(x, y, pixel);
            }
        }
        encode(&img)
    }

    /// An opaque image: a small dark square on a solid white background.
    fn square_on_white_png(size: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba([255, 255, 255, 255]));
        let margin = size / 4;
        for y in margin..(size - margin) {
            for x in margin..(size - margin) {
                img.put_pixel(x, y, image::Rgba([10, 10, 10, 255]));
            }
        }
        encode(&img)
    }

    fn encode(img: &image::RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding a freshly-built test image never fails");
        bytes
    }

    #[test]
    fn analysing_the_same_image_twice_produces_identical_output() {
        let bytes = ring_png(64);
        let path = Path::new("ring.png");

        let first = analyze(path, &bytes).unwrap();
        let second = analyze(path, &bytes).unwrap();

        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap(),
            "analysis must be deterministic",
        );
    }

    #[test]
    fn a_single_opaque_shape_on_a_transparent_background_reports_one_region() {
        let bytes = disc_png(64);
        let analysis = analyze(Path::new("disc.png"), &bytes).unwrap();

        assert_eq!(analysis.region_count, 1);
        assert_eq!(analysis.regions.len(), 1);
        assert_eq!(analysis.regions[0].id, 0);
        assert!(analysis.regions[0].area > 0);
        assert_eq!(analysis.hole_count, 0);
    }

    #[test]
    fn two_disconnected_shapes_report_two_regions_sorted_by_area() {
        let bytes = two_squares_png(60);
        let analysis = analyze(Path::new("squares.png"), &bytes).unwrap();

        assert_eq!(analysis.region_count, 2);
        assert_eq!(analysis.regions.len(), 2);
        // Largest first.
        assert!(analysis.regions[0].area >= analysis.regions[1].area);
        // Ids are stable identifiers, not just positions.
        assert_eq!(analysis.regions[0].id, 0);
        assert_eq!(analysis.regions[1].id, 1);
    }

    #[test]
    fn a_ring_reports_one_region_and_one_hole_enclosed_by_it() {
        let bytes = ring_png(64);
        let analysis = analyze(Path::new("ring.png"), &bytes).unwrap();

        assert_eq!(analysis.region_count, 1, "{:?}", analysis.regions);
        assert_eq!(analysis.hole_count, 1, "{:?}", analysis.holes);
        assert_eq!(analysis.holes[0].enclosed_by, Some(analysis.regions[0].id));
    }

    #[test]
    fn a_transparent_background_is_reported_by_its_alpha_fraction() {
        let bytes = disc_png(64);
        let analysis = analyze(Path::new("disc.png"), &bytes).unwrap();

        assert!(analysis.background.transparent_fraction > 0.0);
        assert!(analysis.background.border_colour.is_none());
        assert_eq!(analysis.background.border_uniformity, 1.0);
    }

    #[test]
    fn an_opaque_flat_background_is_sampled_from_the_border() {
        let bytes = square_on_white_png(64);
        let analysis = analyze(Path::new("square.png"), &bytes).unwrap();

        assert_eq!(analysis.background.transparent_fraction, 0.0);
        assert_eq!(
            analysis.background.border_colour.as_deref(),
            Some("#ffffff")
        );
        assert_eq!(analysis.background.border_uniformity, 1.0);
        assert_eq!(analysis.region_count, 1);
    }

    #[test]
    fn a_horizontally_mirrored_shape_scores_high_horizontal_symmetry() {
        let bytes = disc_png(64);
        let analysis = analyze(Path::new("disc.png"), &bytes).unwrap();

        assert!(
            analysis.symmetry.horizontal > 0.9,
            "{}",
            analysis.symmetry.horizontal
        );
        assert!(
            analysis.symmetry.vertical > 0.9,
            "{}",
            analysis.symmetry.vertical
        );
    }

    #[test]
    fn an_asymmetric_shape_scores_lower_symmetry_than_a_symmetric_one() {
        let symmetric = analyze(Path::new("disc.png"), &disc_png(64)).unwrap();
        let asymmetric = analyze(Path::new("squares.png"), &two_squares_png(64)).unwrap();

        assert!(
            asymmetric.symmetry.horizontal < symmetric.symmetry.horizontal,
            "asymmetric: {}, symmetric: {}",
            asymmetric.symmetry.horizontal,
            symmetric.symmetry.horizontal
        );
    }

    #[test]
    fn dominant_colours_are_sorted_by_coverage_and_capped_at_the_limit() {
        let bytes = square_on_white_png(64);
        let analysis = analyze(Path::new("square.png"), &bytes).unwrap();

        assert!(analysis.dominant_colours.len() <= MAX_DOMINANT_COLOURS);
        // White covers most of the canvas, so it wins the top slot.
        assert_eq!(analysis.dominant_colours[0].colour, "#f0f0f0");
        for pair in analysis.dominant_colours.windows(2) {
            assert!(pair[0].fraction >= pair[1].fraction);
        }
    }

    #[test]
    fn bytes_that_are_not_an_image_report_why() {
        let error = analyze(Path::new("not-an-image"), b"not a png").unwrap_err();

        assert!(matches!(error, Error::AnalysisDecode { .. }), "{error:?}");
    }
}
