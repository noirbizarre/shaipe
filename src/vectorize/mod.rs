//! Measuring a raster reference into vector paths, rather than describing it.
//!
//! An LLM writing `<path d="...">` from a prompt and an image is doing text
//! generation: it can name the shapes present ("hexagon, shackle, keyhole")
//! and estimate rough placement, but it has no mechanism for measuring an
//! actual corner radius or curve tangent from pixels — those numbers are
//! guessed toward "plausible SVG for this kind of icon", not fitted to the
//! image. This module is the other kind of tool: [`trace`] walks pixel
//! contours into cubic bezier paths algorithmically. See ADR 014 for the
//! original decision (a single-colour silhouette) and ADR 020 for this
//! module's later growth: a second, colour-aware mode, structured metadata
//! measured off the traced result itself, and a standalone preview raster.
//!
//! Self-contained and deterministic: bytes in, a typed [`Traced`] report out,
//! no knowledge of `Project`, tools or MCP — mirroring [`crate::render`]'s
//! isolation from the terminal. Given the same bytes and the same
//! [`TraceOptions`] this produces the same output, on any machine — the same
//! property invariant 1 (`AGENTS.md`) already requires of the renderer, now
//! proven for tracing by
//! `tracing_the_same_image_twice_produces_identical_bytes` below.

use std::path::Path;

use image::GenericImageView;
use serde::Serialize;
use vtracer::{ColorImage, Config, Preset};

use crate::error::{Error, Result};

/// How many entries [`Traced::paths`] holds at most. [`Traced::path_count`]
/// still reports the true total, so a caller can tell a clean trace from a
/// fragmented one even when the list itself is capped — the same convention
/// [`crate::analysis::Analysis::region_count`] uses for `MAX_REGIONS`.
const MAX_TRACED_PATHS: usize = 32;

/// Which vtracer frontend traces the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceMode {
    /// One foreground colour vs. one background, by a binary brightness
    /// cutoff (`Clustering::Binary`) — the original, single-shape behaviour
    /// (ADR 014). Good for a single-colour brand mark; not for multi-colour
    /// artwork.
    #[default]
    Silhouette,
    /// Hierarchical colour clustering (`Clustering::ColorCluster`), each
    /// flat-colour region traced as its own path with its own fill. Good for
    /// multi-colour artwork. Nothing decides which colour "is" the
    /// background the way `Silhouette`'s threshold does, so the background
    /// becomes one more traced region rather than being separated out.
    Colour,
}

impl std::fmt::Display for TraceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Silhouette => "silhouette",
            Self::Colour => "colour",
        })
    }
}

/// How a reference image is traced.
///
/// `threshold`/`invert` only affect [`TraceMode::Silhouette`]; `max_colors`
/// only affects [`TraceMode::Colour`] — each mode reads only the field that
/// answers its own ambiguity, and ignores the other mode's rather than
/// refusing it. Deliberately narrow beyond that: not vtracer's full surface
/// (watershed clustering, a fixed palette, mosaic compositing). Extend this
/// further if real usage needs more, the way `set_` was "anticipated but not
/// used" until it was (ADR-007).
#[derive(Debug, Clone, Copy, Default)]
pub struct TraceOptions {
    /// Which frontend traces the reference.
    pub mode: TraceMode,
    /// `Silhouette` only: 0..=255 binary cutoff; pixels darker than this are
    /// traced as foreground. `None` uses vtracer's own default (128).
    pub threshold: Option<u8>,
    /// `Silhouette` only: set when the reference is light artwork on a dark
    /// background, so light pixels are traced as the foreground instead of
    /// dark ones.
    ///
    /// Also chooses what a transparent pixel becomes before tracing:
    /// composited onto white when `false` (a transparent pixel reads as
    /// background, the same as a dark-on-light mark), onto black when `true`
    /// (so it still reads as background once colours are inverted). The
    /// binary frontend judges darkness from RGB alone and does not see alpha
    /// at all, so a transparent pixel left as-is would be classified by
    /// whatever colour its RGB happens to hold underneath the transparency,
    /// not by what it looks like — this is what stops that from being a
    /// silent miscount.
    pub invert: bool,
    /// `Colour` only: caps how many distinct colours the clustering keeps,
    /// merging the rest into their nearest neighbour. `None` lets vtracer's
    /// own clustering decide.
    pub max_colors: Option<usize>,
}

/// A rectangle in the traced SVG's own coordinate space — pixels of the
/// source image, since vtracer's output carries no `viewBox` or transform of
/// its own (its `<svg width height>` are the source image's dimensions
/// directly). Floating-point, and a distinct type from
/// [`crate::analysis::BoundingBox`]: that one counts raster pixels of an
/// input mask, this one measures fitted vector geometry, and forcing the two
/// to share a type would hide that they answer different questions about
/// different things.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BoundingBox {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// One traced `<path>` element.
#[derive(Debug, Clone, Serialize)]
pub struct TracedPath {
    /// Assigned after sorting by bounding-box area, largest first — stable
    /// across truncation to [`MAX_TRACED_PATHS`], the same convention
    /// [`crate::analysis::Region::id`] uses.
    pub id: usize,
    /// The path's bounding box, in source-image pixels.
    pub bounding_box: BoundingBox,
    /// `bounding_box`'s own area (`width * height`) — an upper bound on how
    /// much of the canvas this path actually fills, not an exact fitted-curve
    /// area; no curve integration is performed. A caller needing the exact
    /// figure can rasterise the traced SVG (see [`preview`]) and count
    /// pixels.
    pub area: f64,
    /// The path's fill colour, as `#rrggbb`, when it is a solid colour.
    /// `None` for a gradient, a pattern, or no fill at all — vtracer's own
    /// output never produces the first two today, so this is `Some` in
    /// practice.
    pub fill_colour: Option<String>,
}

/// Everything [`trace`] measured about a reference, alongside the SVG markup
/// itself.
#[derive(Debug, Clone, Serialize)]
pub struct Traced {
    /// Which mode actually produced `svg` — echoed back so a caller reading
    /// only the JSON, not the call that produced it, still knows.
    pub mode: TraceMode,
    /// The traced SVG markup: one `<path>` per silhouette or colour region,
    /// holes cut by winding, no `fill` beyond whatever solid colour the
    /// frontend assigned. Recolouring it, or grafting it into a project, is
    /// the caller's decision — via `write_variant`/`write_svg`, the same as
    /// any other hand-composed markup.
    pub svg: String,
    /// The union of every traced path's bounding box — where, in source
    /// pixels, anything was traced at all.
    pub bounding_box: BoundingBox,
    /// How many paths were traced in total, before `paths` is capped at
    /// [`MAX_TRACED_PATHS`].
    pub path_count: usize,
    /// Traced paths, largest bounding-box area first, capped at
    /// [`MAX_TRACED_PATHS`].
    pub paths: Vec<TracedPath>,
}

/// Decode `bytes` (PNG/JPEG/GIF/WEBP/BMP — the formats
/// [`crate::tools`]'s `get_reference_image` already recognises) and trace it
/// per `options.mode`, then measure the result: an overall bounding box and
/// one [`TracedPath`] per traced path, both derived by parsing the SVG this
/// function just produced (see [`measure`]) rather than by re-deriving them
/// from pixels a second time.
///
/// `path` is only used to name the reference in an error; it is never read.
///
/// # Errors
///
/// Returns [`Error::Decode`] if `bytes` is not a recognisable image, or
/// [`Error::EmptyTrace`] if nothing traced to a path at all — in
/// [`TraceMode::Silhouette`] this means nothing was darker (or, inverted,
/// lighter) than the threshold; in [`TraceMode::Colour`] it practically never
/// fires, since even a flat image traces to one full-canvas region.
pub fn trace(path: &Path, bytes: &[u8], options: TraceOptions) -> Result<Traced> {
    let decoded = image::load_from_memory(bytes).map_err(|source| Error::Decode {
        path: path.to_path_buf(),
        source,
    })?;

    let (width, height) = decoded.dimensions();
    let mut pixels = decoded.to_rgba8().into_raw();

    let config = match options.mode {
        TraceMode::Silhouette => {
            flatten_transparency(&mut pixels, options.invert);
            if options.invert {
                invert_rgb(&mut pixels);
            }
            let mut config = Config::from_preset(Preset::Bw);
            if let Some(threshold) = options.threshold {
                config.binary_threshold = threshold;
            }
            config
        }
        TraceMode::Colour => {
            // Left alone, deliberately: vtracer's colour-cluster frontend
            // does its own transparency keying (`frontend/keying.rs`) once a
            // sampled row is at least a fifth transparent, discarding that
            // background as its own layer. Flattening it onto a backdrop
            // here first would instead bake the backdrop colour into every
            // anti-aliased edge pixel — the opposite of what colour tracing
            // is for.
            let mut config = Config::from_preset(Preset::Poster);
            if let Some(max_colors) = options.max_colors {
                config.max_colors = Some(max_colors);
            }
            config
        }
    };

    let image = ColorImage {
        pixels,
        width: width as usize,
        height: height as usize,
    };

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

    let (bounding_box, path_count, paths) = measure(&svg);

    Ok(Traced {
        mode: options.mode,
        svg,
        bounding_box,
        path_count,
        paths,
    })
}

/// Rasterise `svg` (as produced by [`trace`]) to PNG bytes, at its own native
/// size — no separate width/height parameter, the same restraint ADR-019
/// applied to `compare_reference`'s canvas: one fewer parameter for a model
/// to invent a value for. Takes no [`crate::project::Project`] at all: this
/// is the standalone rasterise path ADR-014's own "Consequences" section
/// named as a reasonable follow-up, and it stays as isolated from a project
/// as [`trace`] itself is.
#[must_use]
pub fn preview(svg: &str) -> Vec<u8> {
    // `usvg::Options::default()` is enough: a traced SVG never carries
    // `<text>` or `<image>`, so no font database or resources directory is
    // ever needed to parse or render it.
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())
        .expect("this module's own SVG output always parses back");
    let size = tree.size();
    let mut pixmap =
        tiny_skia::Pixmap::new(size.width().round() as u32, size.height().round() as u32)
            .expect("a traced SVG's own width/height are never zero");

    // Identity: the traced SVG's own width/height already match its content
    // 1:1, so there is no fit/centring to apply, unlike `render/`'s isolated
    // variants.
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    pixmap
        .encode_png()
        .expect("encoding a freshly rasterised pixmap never fails")
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

/// Measure the SVG `trace` just produced: parse it back with `usvg` — the
/// same library [`crate::render`] already parses every project SVG with, so
/// "how big is what was traced" is answered by the same geometry engine that
/// would later draw it, not re-derived from pixels a second time. Returns the
/// overall bounding box, the true path count, and the (possibly truncated)
/// sorted [`TracedPath`] list.
fn measure(svg: &str) -> (BoundingBox, usize, Vec<TracedPath>) {
    // Parsing this module's own freshly-generated SVG back is assumed to
    // succeed, the same way `Config::build`/`Pipeline::to_svg`'s own
    // unreachable failure modes are treated above: a failure here would be a
    // bug in `trace`, not something a caller did.
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())
        .expect("this module's own SVG output always parses back");

    let mut raw: Vec<(BoundingBox, Option<String>)> = Vec::new();
    collect_paths(tree.root(), &mut raw);

    // Sorted by bounding-box area descending, ties broken by top-left corner
    // — the same determinism-under-truncation discipline
    // `crate::analysis::Region` uses, so `id` survives truncation to
    // `MAX_TRACED_PATHS` unchanged regardless of tree-walk order.
    raw.sort_by(|a, b| {
        area(&b.0)
            .total_cmp(&area(&a.0))
            .then(a.0.y.total_cmp(&b.0.y))
            .then(a.0.x.total_cmp(&b.0.x))
    });

    let path_count = raw.len();
    let bounding_box = union(raw.iter().map(|(bounding_box, _)| *bounding_box));

    let paths = raw
        .into_iter()
        .take(MAX_TRACED_PATHS)
        .enumerate()
        .map(|(id, (bounding_box, fill_colour))| TracedPath {
            id,
            bounding_box,
            area: area(&bounding_box),
            fill_colour,
        })
        .collect();

    (bounding_box, path_count, paths)
}

/// Walk every `<path>` under `group`, recording its bounding box and solid
/// fill colour (if any). Recurses into nested groups — vtracer's own writer
/// does not currently nest paths inside a `<g>`, but nothing here assumes it
/// never will.
fn collect_paths(group: &usvg::Group, out: &mut Vec<(BoundingBox, Option<String>)>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(child) => collect_paths(child, out),
            usvg::Node::Path(path) => {
                let bbox = path.abs_bounding_box();
                let fill_colour = path.fill().and_then(|fill| match fill.paint() {
                    usvg::Paint::Color(colour) => Some(format!(
                        "#{:02x}{:02x}{:02x}",
                        colour.red, colour.green, colour.blue
                    )),
                    _ => None,
                });
                out.push((
                    BoundingBox {
                        x: f64::from(bbox.x()),
                        y: f64::from(bbox.y()),
                        width: f64::from(bbox.width()),
                        height: f64::from(bbox.height()),
                    },
                    fill_colour,
                ));
            }
            // vtracer's own writer emits only `<path>` elements (and the
            // `<svg>` root, which arrives here as the outer `Group`) — never
            // an `<image>` or `<text>` node.
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
}

fn area(bbox: &BoundingBox) -> f64 {
    bbox.width * bbox.height
}

/// The union of every bounding box in `boxes`. A zero rect at the origin
/// when `boxes` is empty — unreachable in practice, since [`measure`] is only
/// ever called after [`trace`] has confirmed the SVG contains at least one
/// `<path>`.
fn union(boxes: impl Iterator<Item = BoundingBox>) -> BoundingBox {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;

    for bbox in boxes {
        any = true;
        min_x = min_x.min(bbox.x);
        min_y = min_y.min(bbox.y);
        max_x = max_x.max(bbox.x + bbox.width);
        max_y = max_y.max(bbox.y + bbox.height);
    }

    if any {
        BoundingBox {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    } else {
        BoundingBox {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }
}

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
        encode(&img)
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
        encode(&img)
    }

    /// Two solid-colour discs, side by side on an opaque white background —
    /// a small margin from the canvas edge and from each other, so each
    /// stays its own connected patch. `n` distinct hues (red, green, blue,
    /// ... a small deterministic palette) laid out left to right, for tests
    /// that need more than two colours.
    fn coloured_discs_png(size: u32, colours: &[[u8; 3]]) -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba([255, 255, 255, 255]));
        let count = colours.len() as u32;
        let cell = size / count;
        let radius = f64::from(cell) / 2.0 - 4.0;
        for (index, &[r, g, b]) in colours.iter().enumerate() {
            let cx = f64::from(index as u32 * cell) + f64::from(cell) / 2.0;
            let cy = f64::from(size) / 2.0;
            for y in 0..size {
                for x in 0..size {
                    let dx = f64::from(x) - cx;
                    let dy = f64::from(y) - cy;
                    if (dx * dx + dy * dy).sqrt() <= radius {
                        img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                    }
                }
            }
        }
        encode(&img)
    }

    /// A grid of small, disjoint filled squares on a transparent background —
    /// `rows * cols` disconnected regions, for tests that need more than
    /// `MAX_TRACED_PATHS` of them.
    fn grid_of_squares_png(size: u32, rows: u32, cols: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::new(size, size);
        let cell_w = size / cols;
        let cell_h = size / rows;
        let square = (cell_w.min(cell_h)) / 2;
        for row in 0..rows {
            for col in 0..cols {
                let cx = col * cell_w + cell_w / 2;
                let cy = row * cell_h + cell_h / 2;
                for y in cy.saturating_sub(square / 2)..(cy + square / 2).min(size) {
                    for x in cx.saturating_sub(square / 2)..(cx + square / 2).min(size) {
                        img.put_pixel(x, y, image::Rgba([10, 10, 10, 255]));
                    }
                }
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
    fn tracing_the_same_image_twice_produces_identical_bytes() {
        let bytes = ring_png(64, None);
        let path = Path::new("ring.png");

        let first = trace(path, &bytes, TraceOptions::default()).unwrap();
        let second = trace(path, &bytes, TraceOptions::default()).unwrap();

        assert_eq!(first.svg, second.svg, "tracing must be deterministic");
    }

    #[test]
    fn a_ring_traces_to_a_single_path_with_a_hole() {
        let bytes = ring_png(64, None);
        let traced = trace(Path::new("ring.png"), &bytes, TraceOptions::default()).unwrap();

        // One region (the ring), fitted as one path whose fill-rule cuts the
        // inner circle as a hole, the same shape lock.svg turned out to be.
        assert_eq!(traced.path_count, 1, "{}", traced.svg);
        assert_eq!(traced.paths.len(), 1);
    }

    #[test]
    fn a_ring_reports_a_bounding_box_matching_its_own_outer_radius() {
        let size = 64;
        let bytes = ring_png(size, None);
        let traced = trace(Path::new("ring.png"), &bytes, TraceOptions::default()).unwrap();

        // `ring_png` draws the outer circle at `center - 4.0` radius, so the
        // ring spans roughly `[4, size - 4]` on both axes.
        let bbox = traced.paths[0].bounding_box;
        assert!(bbox.x > 0.0 && bbox.x < 10.0, "{bbox:?}");
        assert!(bbox.y > 0.0 && bbox.y < 10.0, "{bbox:?}");
        assert!(bbox.width > f64::from(size) - 20.0, "{bbox:?}");
        assert!(bbox.height > f64::from(size) - 20.0, "{bbox:?}");
        assert_eq!(
            (traced.bounding_box.x, traced.bounding_box.y),
            (bbox.x, bbox.y),
            "the only path's bbox is the whole trace's bbox"
        );
    }

    #[test]
    fn a_silhouette_path_reports_its_fill_colour() {
        let bytes = ring_png(64, None);
        let traced = trace(Path::new("ring.png"), &bytes, TraceOptions::default()).unwrap();

        assert_eq!(
            traced.paths[0].fill_colour.as_deref(),
            Some("#000000"),
            "{:?}",
            traced.paths[0]
        );
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

        assert_eq!(traced_transparent.svg, traced_white.svg);
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
        assert_ne!(not_inverted.svg, inverted.svg);
        assert!(inverted.path_count > 0);
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
        let bytes = encode(&img);

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
        assert!(high.path_count > 0);
    }

    #[test]
    fn a_blank_image_reports_that_nothing_was_traced() {
        let size = 32;
        let img = image::RgbaImage::from_pixel(size, size, image::Rgba([255, 255, 255, 255]));
        let bytes = encode(&img);

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

    #[test]
    fn colour_mode_traces_a_two_colour_image_into_multiple_paths_with_distinct_fills() {
        let bytes = coloured_discs_png(96, &[[220, 30, 30], [30, 90, 220]]);
        let traced = trace(
            Path::new("discs.png"),
            &bytes,
            TraceOptions {
                mode: TraceMode::Colour,
                ..TraceOptions::default()
            },
        )
        .unwrap();

        assert_eq!(traced.mode, TraceMode::Colour);
        // At least the two discs and the white background, each its own
        // colour-clustered region.
        assert!(traced.path_count >= 2, "{}", traced.svg);
        let colours: std::collections::BTreeSet<_> = traced
            .paths
            .iter()
            .filter_map(|path| path.fill_colour.clone())
            .collect();
        assert!(
            colours.len() >= 2,
            "expected at least two distinct fills, got {colours:?}"
        );
    }

    #[test]
    fn colour_tracing_is_deterministic() {
        let bytes = coloured_discs_png(64, &[[220, 30, 30], [30, 90, 220], [40, 180, 60]]);
        let options = TraceOptions {
            mode: TraceMode::Colour,
            ..TraceOptions::default()
        };

        let first = trace(Path::new("discs.png"), &bytes, options).unwrap();
        let second = trace(Path::new("discs.png"), &bytes, options).unwrap();

        assert_eq!(
            first.svg, second.svg,
            "colour tracing must be deterministic"
        );
    }

    #[test]
    fn max_colors_bounds_how_many_distinct_fills_colour_mode_keeps() {
        let bytes = coloured_discs_png(
            120,
            &[[220, 30, 30], [30, 90, 220], [40, 180, 60], [220, 180, 30]],
        );

        let unbounded = trace(
            Path::new("discs.png"),
            &bytes,
            TraceOptions {
                mode: TraceMode::Colour,
                ..TraceOptions::default()
            },
        )
        .unwrap();
        let bounded = trace(
            Path::new("discs.png"),
            &bytes,
            TraceOptions {
                mode: TraceMode::Colour,
                max_colors: Some(2),
                ..TraceOptions::default()
            },
        )
        .unwrap();

        let distinct = |traced: &Traced| -> usize {
            traced
                .paths
                .iter()
                .filter_map(|path| path.fill_colour.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        };

        assert!(
            distinct(&bounded) <= distinct(&unbounded),
            "bounded: {}, unbounded: {}",
            distinct(&bounded),
            distinct(&unbounded)
        );
    }

    #[test]
    fn path_count_reports_the_true_total_even_when_paths_is_capped() {
        // A 7x7 grid of disjoint squares — 49 connected components, well
        // over `MAX_TRACED_PATHS` (32).
        let bytes = grid_of_squares_png(140, 7, 7);
        let traced = trace(Path::new("grid.png"), &bytes, TraceOptions::default()).unwrap();

        assert_eq!(traced.path_count, 49, "{}", traced.svg);
        assert_eq!(traced.paths.len(), MAX_TRACED_PATHS);
    }

    #[test]
    fn preview_rasterises_to_a_png_matching_the_source_dimensions() {
        let size = 48;
        let bytes = ring_png(size, None);
        let traced = trace(Path::new("ring.png"), &bytes, TraceOptions::default()).unwrap();

        let png = preview(&traced.svg);
        let decoded = image::load_from_memory(&png).expect("preview always encodes a valid PNG");

        assert_eq!(decoded.dimensions(), (size, size));
    }
}
