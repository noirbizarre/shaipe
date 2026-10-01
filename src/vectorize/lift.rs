//! Lifting the regions that colour tracing would destroy out of the trace.
//!
//! A colour trace of a gradient is not wrong so much as useless: it stacks ten
//! or a hundred flat layers, each a slightly different colour, and the fade
//! that was one fact about one region becomes noise in the geometry. A
//! translucent fill fares worse, because a traced fill has no alpha at all.
//!
//! So the regions whose appearance [`crate::analysis`] already measured as a
//! gradient, or as a flat colour that is drawn translucent, are *lifted*:
//!
//! 1. their pixels are blanked before the colour trace, so it neither
//!    fragments them nor spends its layers on them;
//! 2. each region's silhouette is traced separately as one path, which is
//!    plain geometry and needs nothing from the appearance;
//! 3. that path is painted with the measured gradient or opacity.
//!
//! Geometry and appearance stay separable: the path's outline is the trace's,
//! the paint is the analysis's, and a caller who distrusts the paint can
//! recolour the path without touching its shape. Every region that is *not*
//! lifted is left exactly as a plain colour trace would have it. A region
//! that varies without being a gradient is reported as a [`Fallback`] so the
//! caller knows that the flat layers it sees are all there is.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use vtracer::{ColorImage, Config, Preset};

use crate::analysis::OPAQUE_FLOOR;
use crate::analysis::{Analysis, Fill, GradientStop, NO_REGION, Opacity, Region};
use crate::error::{Error, Result};

/// Interior opacity below which a flat region counts as drawn translucent.
/// Derived from the analysis's own line between opaque and not, so a region
/// is never "opaque" in `AlphaSummary` yet lifted as translucent.
const TRANSLUCENT_BELOW: f64 = OPAQUE_FLOOR as f64 / 255.0;

/// Fewest pixels a flat region needs before its translucency is believed. A
/// hairline is nothing but anti-aliased edge, whose alpha is partial by
/// construction; it is not a translucent fill, and lifting it would swap a
/// faithful thin path for a fuzzy one.
const MIN_TRANSLUCENT_AREA: u64 = 64;

/// The prefix of the `id` a lifted path carries, followed by its region id.
/// [`region_of`] reads it back, which is how a path measured from the SVG is
/// tied to the analysis region it came from.
const PATH_ID_PREFIX: &str = "shaipe-region-";

/// The prefix of a lifted region's gradient `id`.
const FILL_ID_PREFIX: &str = "shaipe-fill-";

/// How a lifted path is painted, kept beside the geometry rather than inside
/// it.
#[derive(Debug, Clone, Serialize)]
pub struct PathAppearance {
    /// The measured fill: a gradient with its stops, or a flat colour.
    pub fill: Fill,
    /// The region's opacity, apart from its colour. For a gradient this is
    /// what the stops' own opacities average to; the stops are authoritative.
    pub opacity: Opacity,
}

/// A region left as flat colour layers although its colour varies, and why.
///
/// Not an error: a region of several touching flat colours is ordinary, and
/// the layers are the right answer for it. It is the explicit statement that
/// no gradient could be vouched for, so the absence of one is not a silent
/// omission.
#[derive(Debug, Clone, Serialize)]
pub struct Fallback {
    /// The [`Region::id`] left as flat layers.
    pub region_id: usize,
    /// What was measured, and what that leaves the caller to do.
    pub reason: String,
}

/// What [`decide`] chose for every region.
pub(super) struct Decision {
    /// The regions to lift, in region order.
    pub lifts: Vec<Lift>,
    /// The regions that vary but could not be lifted.
    pub fallbacks: Vec<Fallback>,
}

/// One region chosen for lifting.
pub(super) struct Lift {
    pub region_id: usize,
    pub appearance: PathAppearance,
}

/// What [`render`] produced for the lifted regions.
pub(super) struct Rendered {
    /// `<defs>` and `<path>` markup, to sit before the closing `</svg>`.
    pub markup: String,
    /// Each lifted region's appearance, by region id.
    pub appearances: BTreeMap<usize, PathAppearance>,
}

/// Choose, from the measured appearance, which regions to lift.
pub(super) fn decide(analysis: &Analysis) -> Decision {
    let mut lifts = Vec::new();
    let mut fallbacks = Vec::new();

    for (region, appearance) in analysis.regions.iter().zip(&analysis.appearance.regions) {
        match &appearance.fill {
            Fill::LinearGradient { .. } | Fill::RadialGradient { .. } => lifts.push(Lift {
                region_id: region.id,
                appearance: PathAppearance {
                    fill: appearance.fill.clone(),
                    opacity: appearance.opacity.clone(),
                },
            }),
            Fill::Flat { .. } if is_translucent(region, &appearance.opacity) => {
                lifts.push(Lift {
                    region_id: region.id,
                    appearance: PathAppearance {
                        fill: appearance.fill.clone(),
                        opacity: appearance.opacity.clone(),
                    },
                });
            }
            Fill::Varied {
                spread,
                linear_fit,
                radial_fit,
                ..
            } => fallbacks.push(Fallback {
                region_id: region.id,
                reason: format!(
                    "colour varies (spread {spread:.0}) but is not one gradient (linear fit \
                     {linear_fit:.2}, radial fit {radial_fit:.2}), so it is left as flat colour \
                     layers; build its fill from get_reference_analysis instead if it is one"
                ),
            }),
            Fill::Flat { .. } => {}
        }
    }

    Decision { lifts, fallbacks }
}

/// Whether a flat region is drawn translucent rather than merely soft-edged.
fn is_translucent(region: &Region, opacity: &Opacity) -> bool {
    // The *highest* interior opacity: one opaque pixel in the interior is
    // enough to say the fill is not translucent, whatever its average.
    opacity.max < TRANSLUCENT_BELOW && region.area >= MIN_TRANSLUCENT_AREA
}

/// Paint every lifted region's pixels with what the colour trace should see
/// instead: the image's own background when it has one, and nothing when it
/// does not. Leaving the region in would fragment it; leaving a hole of some
/// third colour would trace as a phantom shape.
pub(super) fn blank(
    pixels: &mut [u8],
    region_map: &[u32],
    lifts: &[Lift],
    background: Option<[u8; 3]>,
) {
    let replacement = match background {
        Some([r, g, b]) => [r, g, b, 255],
        None => [0, 0, 0, 0],
    };
    let lifted: Vec<u32> = lifts.iter().map(|lift| lift.region_id as u32).collect();
    for (pixel, &region) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(region_map) {
        if region != NO_REGION && lifted.contains(&region) {
            *pixel = replacement;
        }
    }
}

/// Trace each lifted region's silhouette and paint it.
///
/// # Errors
///
/// [`Error::Trace`] if vtracer refuses a mask, which it does not for a
/// configuration this module builds.
pub(super) fn render(
    path: &Path,
    lifts: &[Lift],
    region_map: &[u32],
    width: usize,
    height: usize,
) -> Result<Rendered> {
    let mut definitions = String::new();
    let mut paths = String::new();
    let mut appearances = BTreeMap::new();

    for lift in lifts {
        let paint = paint_for(lift, &mut definitions);
        let outlines = silhouette(path, region_map, lift.region_id, width, height)?;
        for (index, outline) in outlines.iter().enumerate() {
            let id = if index == 0 {
                format!("{PATH_ID_PREFIX}{}", lift.region_id)
            } else {
                format!("{PATH_ID_PREFIX}{}-{index}", lift.region_id)
            };
            // The mask is drawn in black, and `fill="#000000"` is how the
            // frontend wrote that; it is the one attribute to swap.
            paths.push_str(&outline.replace("fill=\"#000000\"", &format!("id=\"{id}\" {paint}")));
            paths.push('\n');
        }
        appearances.insert(lift.region_id, lift.appearance.clone());
    }

    let markup = if definitions.is_empty() {
        paths
    } else {
        format!("<defs>\n{definitions}</defs>\n{paths}")
    };
    Ok(Rendered {
        markup,
        appearances,
    })
}

/// The region a lifted path came from, read back from its `id`.
pub(super) fn region_of(id: &str) -> Option<usize> {
    id.strip_prefix(PATH_ID_PREFIX)?
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// The paint attributes for `lift`, adding a gradient definition to
/// `definitions` when it needs one.
fn paint_for(lift: &Lift, definitions: &mut String) -> String {
    let id = format!("{FILL_ID_PREFIX}{}", lift.region_id);
    match &lift.appearance.fill {
        Fill::LinearGradient {
            start, end, stops, ..
        } => {
            definitions.push_str(&format!(
                "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" \
                 y1=\"{}\" x2=\"{}\" y2=\"{}\">{}</linearGradient>\n",
                number(start.x),
                number(start.y),
                number(end.x),
                number(end.y),
                stop_markup(stops),
            ));
            format!("fill=\"url(#{id})\"")
        }
        Fill::RadialGradient {
            centre,
            radius,
            stops,
            ..
        } => {
            definitions.push_str(&format!(
                "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" \
                 cy=\"{}\" r=\"{}\">{}</radialGradient>\n",
                number(centre.x),
                number(centre.y),
                number(*radius),
                stop_markup(stops),
            ));
            format!("fill=\"url(#{id})\"")
        }
        // `Varied` is never lifted; it is named only so the match is total,
        // and painting it as its mean colour is the honest reading if it were.
        Fill::Flat { colour, .. }
        | Fill::Varied {
            mean_colour: colour,
            ..
        } => format!(
            "fill=\"{colour}\" fill-opacity=\"{}\"",
            number(lift.appearance.opacity.mean)
        ),
    }
}

/// A gradient's `<stop>` elements. Opacity is written only where it is not
/// one, so an opaque gradient stays as small as a hand-written one.
fn stop_markup(stops: &[GradientStop]) -> String {
    stops
        .iter()
        .map(|stop| {
            let opacity = if stop.opacity < 1.0 {
                format!(" stop-opacity=\"{}\"", number(stop.opacity))
            } else {
                String::new()
            };
            format!(
                "<stop offset=\"{}\" stop-color=\"{}\"{opacity}/>",
                number(stop.offset),
                stop.colour
            )
        })
        .collect()
}

/// A number with at most two decimals and no trailing zeros. Fixed here so
/// the same measurement always writes the same bytes.
fn number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    // `-0.001` rounds to `-0.00`, which trims to `-0`.
    if text == "-0" || text.is_empty() {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// The `<path>` elements tracing one region's mask gives, as markup. Black on
/// white and binary, whatever the region's colour was: this is geometry.
fn silhouette(
    path: &Path,
    region_map: &[u32],
    region_id: usize,
    width: usize,
    height: usize,
) -> Result<Vec<String>> {
    let pixels: Vec<u8> = region_map
        .iter()
        .flat_map(|&region| {
            if region == region_id as u32 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        })
        .collect();
    let image = ColorImage {
        pixels,
        width,
        height,
    };
    let svg = Config::from_preset(Preset::Bw)
        .build()
        .and_then(|pipeline| pipeline.to_svg(&image))
        .map_err(|source| Error::Trace {
            path: path.to_path_buf(),
            reason: source.to_string(),
        })?;
    Ok(paths_of(&svg))
}

/// Every `<path .../>` element in `svg`, verbatim.
fn paths_of(svg: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find("<path") {
        let Some(length) = rest[start..].find("/>") else {
            break;
        };
        found.push(rest[start..start + length + 2].to_owned());
        rest = &rest[start + length + 2..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lifted_path_is_tied_back_to_its_region() {
        assert_eq!(region_of("shaipe-region-3"), Some(3));
        assert_eq!(region_of("shaipe-region-12-1"), Some(12));
        assert_eq!(region_of("path7"), None);
        assert_eq!(region_of("shaipe-region-x"), None);
    }

    #[test]
    fn numbers_are_written_short_and_without_a_negative_zero() {
        assert_eq!(number(12.0), "12");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(1.0 / 3.0), "0.33");
        assert_eq!(number(-0.001), "0");
    }
}
