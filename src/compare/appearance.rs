//! How a render is *filled*, against how the reference is — the diagnostics
//! that geometry cannot give. See ADR 027.
//!
//! Nothing is measured here. [`crate::analysis`] already describes each
//! image's fills, opacity and strokes; this lines the two descriptions up,
//! region against region, and says where they disagree. Like the rest of
//! [`super`], nothing is folded into a score: a render can match on shape and
//! miss on fill, and [`AppearanceComparison::findings`] says which.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::analysis::{AlphaSummary, Analysis, Fill, GradientStop, NO_REGION, RegionAppearance};

/// Mean channel difference (0-1) above which two colours are called different.
const COLOUR_TOLERANCE: f64 = 0.1;
/// Angle difference, in degrees, above which two gradient axes are different.
const ANGLE_TOLERANCE: f64 = 10.0;
/// Change in a fraction of the canvas, or in a region's opacity, that is
/// worth reporting.
const ALPHA_TOLERANCE: f64 = 0.05;
/// Relative change in a radial gradient's radius, or a stroke's width.
const RATIO_TOLERANCE: f64 = 0.25;
/// Shared offsets at which two stop lists are sampled to compare them.
const STOP_SAMPLES: usize = 5;

/// Fill, opacity and stroke differences between a reference and a render.
#[derive(Debug, Clone, Serialize)]
pub struct AppearanceComparison {
    /// Canvas-wide transparency, both sides and their difference.
    pub alpha: AlphaComparison,
    /// One entry per reference region, in the reference's order.
    pub regions: Vec<RegionComparison>,
    /// Only the disagreements, in words, each saying what to change. Empty
    /// when the two are filled alike.
    pub findings: Vec<String>,
}

/// Transparency of the whole canvas, reference against render.
#[derive(Debug, Clone, Serialize)]
pub struct AlphaComparison {
    /// The reference's own summary.
    pub reference: AlphaSummary,
    /// The render's own summary.
    pub render: AlphaSummary,
    /// Render minus reference, for the transparent fraction.
    pub transparent_delta: f64,
    /// Render minus reference, for the translucent fraction.
    pub translucent_delta: f64,
    /// Render minus reference, for the opaque fraction.
    pub opaque_delta: f64,
    /// Render minus reference, for the drawn (interior) translucency.
    pub interior_translucent_delta: f64,
}

/// One reference region against the render region covering most of it.
#[derive(Debug, Clone, Serialize)]
pub struct RegionComparison {
    /// The reference region's id.
    pub reference_region_id: usize,
    /// The render region sharing the most pixels with it, or `None` when the
    /// render paints nothing there.
    pub render_region_id: Option<usize>,
    /// Share of the reference region's pixels the matched render region
    /// covers.
    pub overlap: f64,
    /// The reference fill's kind: `flat`, `linear_gradient`, `radial_gradient`
    /// or `varied`.
    pub reference_fill: &'static str,
    /// The render fill's kind, `None` with no matched region.
    pub render_fill: Option<&'static str>,
    /// Whether the two kinds agree.
    pub kind_matches: bool,
    /// Mean colour distance, 0-1: the flat colours, or the stops sampled at
    /// shared offsets. `None` unless both fills have colours to compare.
    pub colour_difference: Option<f64>,
    /// Difference between the two linear axes, in degrees, 0-90 (an axis has
    /// no sign; a reversed gradient shows in `colour_difference`).
    pub angle_difference_degrees: Option<f64>,
    /// Distance between the two radial centres, in pixels.
    pub centre_offset: Option<f64>,
    /// Render radius over reference radius.
    pub radius_ratio: Option<f64>,
    /// Render mean opacity minus reference mean opacity.
    pub opacity_delta: Option<f64>,
    /// Render stroke width over reference stroke width, when both are
    /// strokes.
    pub stroke_width_ratio: Option<f64>,
}

/// Line the two images' appearance up and report where they differ.
///
/// `*_map` are the per-pixel region ids from `analyze_with_regions`, the
/// same length.
pub(crate) fn compare_appearance(
    reference: &Analysis,
    reference_map: &[u32],
    render: &Analysis,
    render_map: &[u32],
) -> AppearanceComparison {
    let (r, g) = (&reference.appearance.alpha, &render.appearance.alpha);
    let alpha = AlphaComparison {
        reference: r.clone(),
        render: g.clone(),
        transparent_delta: g.transparent_fraction - r.transparent_fraction,
        translucent_delta: g.translucent_fraction - r.translucent_fraction,
        opaque_delta: g.opaque_fraction - r.opaque_fraction,
        interior_translucent_delta: g.interior_translucent_fraction
            - r.interior_translucent_fraction,
    };

    let mut findings = Vec::new();
    if alpha.interior_translucent_delta.abs() > ALPHA_TOLERANCE {
        let which = if alpha.interior_translucent_delta < 0.0 {
            "is more solid"
        } else {
            "is more translucent"
        };
        findings.push(format!(
            "the render {which} than the reference: {:.0}% of the canvas is drawn translucent \
             in the reference, {:.0}% in the render; adjust fill-opacity",
            r.interior_translucent_fraction * 100.0,
            g.interior_translucent_fraction * 100.0,
        ));
    }

    let regions = reference
        .appearance
        .regions
        .iter()
        .map(|region| {
            let matched = best_match(region.region_id, reference_map, render_map);
            let render_region = matched.and_then(|(id, _)| {
                render
                    .appearance
                    .regions
                    .iter()
                    .find(|candidate| candidate.region_id == id)
            });
            let area = reference
                .regions
                .iter()
                .find(|candidate| candidate.id == region.region_id)
                .map_or(0, |candidate| candidate.area);
            let overlap = match matched {
                Some((_, shared)) if area > 0 => shared as f64 / area as f64,
                _ => 0.0,
            };
            let entry = compare_region(region, render_region, overlap);
            describe(&entry, region, render_region, &mut findings);
            entry
        })
        .collect();

    AppearanceComparison {
        alpha,
        regions,
        findings,
    }
}

/// The render region sharing the most pixels with reference region `id`, and
/// how many it shares.
fn best_match(id: usize, reference_map: &[u32], render_map: &[u32]) -> Option<(usize, u64)> {
    // A BTreeMap so a tie breaks towards the lowest id on every machine.
    let mut shared = BTreeMap::<u32, u64>::new();
    for (&a, &b) in reference_map.iter().zip(render_map) {
        if a != NO_REGION && a as usize == id && b != NO_REGION {
            *shared.entry(b).or_default() += 1;
        }
    }
    shared
        .into_iter()
        .fold(
            None,
            |best: Option<(u32, u64)>, (region, count)| match best {
                Some((_, top)) if top >= count => best,
                _ => Some((region, count)),
            },
        )
        .map(|(region, count)| (region as usize, count))
}

fn kind(fill: &Fill) -> &'static str {
    match fill {
        Fill::Flat { .. } => "flat",
        Fill::LinearGradient { .. } => "linear_gradient",
        Fill::RadialGradient { .. } => "radial_gradient",
        Fill::Varied { .. } => "varied",
    }
}

fn compare_region(
    reference: &RegionAppearance,
    render: Option<&RegionAppearance>,
    overlap: f64,
) -> RegionComparison {
    let mut entry = RegionComparison {
        reference_region_id: reference.region_id,
        render_region_id: render.map(|r| r.region_id),
        overlap,
        reference_fill: kind(&reference.fill),
        render_fill: render.map(|r| kind(&r.fill)),
        kind_matches: false,
        colour_difference: None,
        angle_difference_degrees: None,
        centre_offset: None,
        radius_ratio: None,
        opacity_delta: None,
        stroke_width_ratio: None,
    };
    let Some(render) = render else {
        return entry;
    };
    entry.kind_matches = entry.render_fill == Some(entry.reference_fill);
    entry.opacity_delta = Some(render.opacity.mean - reference.opacity.mean);
    if let (Some(a), Some(b)) = (&reference.stroke, &render.stroke) {
        entry.stroke_width_ratio = (a.width > 0.0).then(|| b.width / a.width);
    }

    match (&reference.fill, &render.fill) {
        (Fill::Flat { colour: a, .. }, Fill::Flat { colour: b, .. }) => {
            entry.colour_difference = Some(hex_distance(a, b));
        }
        (
            Fill::LinearGradient {
                angle_degrees: a,
                stops: sa,
                ..
            },
            Fill::LinearGradient {
                angle_degrees: b,
                stops: sb,
                ..
            },
        ) => {
            // An axis has no sign: 179 and 1 degrees are nearly the same line.
            let turn = (a - b).abs() % 180.0;
            entry.angle_difference_degrees = Some(turn.min(180.0 - turn));
            entry.colour_difference = Some(stops_distance(sa, sb));
        }
        (
            Fill::RadialGradient {
                centre: ca,
                radius: ra,
                stops: sa,
                ..
            },
            Fill::RadialGradient {
                centre: cb,
                radius: rb,
                stops: sb,
                ..
            },
        ) => {
            entry.centre_offset = Some((ca.x - cb.x).hypot(ca.y - cb.y));
            entry.radius_ratio = (*ra > 0.0).then(|| rb / ra);
            entry.colour_difference = Some(stops_distance(sa, sb));
        }
        _ => {}
    }
    entry
}

/// Turn one region's numbers into sentences, when they disagree.
fn describe(
    entry: &RegionComparison,
    reference: &RegionAppearance,
    render: Option<&RegionAppearance>,
    findings: &mut Vec<String>,
) {
    let id = entry.reference_region_id;
    let Some(render) = render else {
        findings.push(format!(
            "reference region {id} ({}) has nothing painted over it in the render",
            entry.reference_fill
        ));
        return;
    };
    if !entry.kind_matches {
        findings.push(format!(
            "region {id}: the reference is {}, the render is {}; {}",
            say(&reference.fill),
            say(&render.fill),
            advice(&reference.fill),
        ));
        return;
    }
    if let Some(angle) = entry
        .angle_difference_degrees
        .filter(|a| *a > ANGLE_TOLERANCE)
    {
        findings.push(format!(
            "region {id}: the gradient direction differs by {angle:.0} degrees ({}, render {}); \
             change the gradient's x1/y1/x2/y2",
            say(&reference.fill),
            say(&render.fill),
        ));
    }
    if let Some(offset) = entry.centre_offset.filter(|o| *o > 1.0) {
        findings.push(format!(
            "region {id}: the radial gradient's centre is {offset:.1}px from the reference's"
        ));
    }
    if let Some(ratio) = entry
        .radius_ratio
        .filter(|r| (r - 1.0).abs() > RATIO_TOLERANCE)
    {
        findings.push(format!(
            "region {id}: the radial gradient's radius is {ratio:.2} times the reference's"
        ));
    }
    if let Some(distance) = entry.colour_difference.filter(|d| *d > COLOUR_TOLERANCE) {
        findings.push(format!(
            "region {id}: the colours differ by {:.0}% ({}, render {}); recolour the {}",
            distance * 100.0,
            say(&reference.fill),
            say(&render.fill),
            if entry.reference_fill == "flat" {
                "fill"
            } else {
                "stops"
            },
        ));
    }
    if let Some(delta) = entry.opacity_delta.filter(|d| d.abs() > ALPHA_TOLERANCE) {
        findings.push(format!(
            "region {id}: the render's opacity is {delta:+.2} against the reference's \
             {:.2}; set fill-opacity",
            reference.opacity.mean
        ));
    }
    if let Some(ratio) = entry
        .stroke_width_ratio
        .filter(|r| (r - 1.0).abs() > RATIO_TOLERANCE)
    {
        findings.push(format!(
            "region {id}: the stroke is {ratio:.2} times as wide as the reference's"
        ));
    }
}

fn advice(reference: &Fill) -> &'static str {
    match reference {
        Fill::Flat { .. } => "use a single flat fill",
        Fill::LinearGradient { .. } => "paint it with a linearGradient",
        Fill::RadialGradient { .. } => "paint it with a radialGradient",
        Fill::Varied { .. } => "it is several colours or texture; split it into separate shapes",
    }
}

fn say(fill: &Fill) -> String {
    match fill {
        Fill::Flat { colour, .. } => format!("flat {colour}"),
        Fill::LinearGradient {
            angle_degrees,
            stops,
            ..
        } => format!(
            "a linear_gradient at {angle_degrees:.0} degrees {}",
            ends(stops)
        ),
        Fill::RadialGradient { stops, .. } => format!("a radial_gradient {}", ends(stops)),
        Fill::Varied { mean_colour, .. } => format!("varied around {mean_colour}"),
    }
}

fn ends(stops: &[GradientStop]) -> String {
    match (stops.first(), stops.last()) {
        (Some(a), Some(b)) => format!("({} to {})", a.colour, b.colour),
        _ => String::new(),
    }
}

fn rgb(hex: &str) -> [f64; 3] {
    let channel = |i: usize| {
        hex.get(1 + i * 2..3 + i * 2)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .map_or(0.0, f64::from)
    };
    [channel(0), channel(1), channel(2)]
}

/// Mean absolute channel difference of two `#rrggbb` colours, 0-1.
pub(super) fn hex_distance(a: &str, b: &str) -> f64 {
    let (a, b) = (rgb(a), rgb(b));
    (0..3).map(|i| (a[i] - b[i]).abs()).sum::<f64>() / (3.0 * 255.0)
}

/// The colour of a stop list at `offset`, interpolated linearly.
fn colour_at(stops: &[GradientStop], offset: f64) -> [f64; 3] {
    let Some(first) = stops.first() else {
        return [0.0; 3];
    };
    if offset <= first.offset {
        return rgb(&first.colour);
    }
    for pair in stops.windows(2) {
        if offset <= pair[1].offset {
            let span = (pair[1].offset - pair[0].offset).max(f64::EPSILON);
            let t = (offset - pair[0].offset) / span;
            let (a, b) = (rgb(&pair[0].colour), rgb(&pair[1].colour));
            return [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
        }
    }
    stops.last().map_or([0.0; 3], |s| rgb(&s.colour))
}

/// Mean colour distance of two stop lists, sampled at shared offsets so
/// lists of different lengths still compare.
fn stops_distance(a: &[GradientStop], b: &[GradientStop]) -> f64 {
    let total: f64 = (0..STOP_SAMPLES)
        .map(|i| {
            let offset = i as f64 / (STOP_SAMPLES - 1) as f64;
            let (x, y) = (colour_at(a, offset), colour_at(b, offset));
            (0..3).map(|c| (x[c] - y[c]).abs()).sum::<f64>() / (3.0 * 255.0)
        })
        .sum();
    total / STOP_SAMPLES as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Opacity, Point};

    fn stop(offset: f64, colour: &str) -> GradientStop {
        GradientStop {
            offset,
            colour: colour.into(),
            opacity: 1.0,
        }
    }

    fn region(fill: Fill) -> RegionAppearance {
        RegionAppearance {
            region_id: 0,
            fill,
            opacity: Opacity {
                min: 1.0,
                mean: 1.0,
                max: 1.0,
            },
            stroke: None,
        }
    }

    fn linear(angle: f64, from: &str, to: &str) -> Fill {
        Fill::LinearGradient {
            start: Point { x: 0.0, y: 0.0 },
            end: Point { x: 1.0, y: 0.0 },
            angle_degrees: angle,
            stops: vec![stop(0.0, from), stop(1.0, to)],
            fit: 1.0,
        }
    }

    fn flat(colour: &str) -> Fill {
        Fill::Flat {
            colour: colour.into(),
            spread: 0.0,
        }
    }

    fn findings(reference: Fill, render: Fill) -> Vec<String> {
        let (a, b) = (region(reference), region(render));
        let entry = compare_region(&a, Some(&b), 1.0);
        let mut out = Vec::new();
        describe(&entry, &a, Some(&b), &mut out);
        out
    }

    #[test]
    fn identical_gradients_report_no_findings() {
        let out = findings(
            linear(0.0, "#ff0000", "#0000ff"),
            linear(0.0, "#ff0000", "#0000ff"),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_flat_render_of_a_gradient_says_to_use_a_gradient() {
        let out = findings(linear(0.0, "#ff0000", "#0000ff"), flat("#800080"));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("linear_gradient") && out[0].contains("linearGradient"));
    }

    #[test]
    fn a_reversed_gradient_differs_in_its_colours() {
        let out = findings(
            linear(0.0, "#ff0000", "#0000ff"),
            linear(0.0, "#0000ff", "#ff0000"),
        );
        assert!(out.iter().any(|f| f.contains("colours differ")));
    }

    #[test]
    fn a_rotated_gradient_reports_the_angle_difference() {
        let out = findings(
            linear(0.0, "#ff0000", "#0000ff"),
            linear(90.0, "#ff0000", "#0000ff"),
        );
        assert!(out.iter().any(|f| f.contains("differs by 90 degrees")));
    }

    #[test]
    fn a_linear_gradient_against_a_radial_one_is_a_kind_mismatch() {
        let radial = Fill::RadialGradient {
            centre: Point { x: 0.0, y: 0.0 },
            radius: 10.0,
            stops: vec![stop(0.0, "#ff0000"), stop(1.0, "#0000ff")],
            fit: 1.0,
        };
        let out = findings(linear(0.0, "#ff0000", "#0000ff"), radial);
        assert!(out[0].contains("radial_gradient"));
    }

    #[test]
    fn an_unmatched_region_is_reported_with_no_render_region() {
        let a = region(flat("#ff0000"));
        let entry = compare_region(&a, None, 0.0);
        assert_eq!(entry.render_region_id, None);
        let mut out = Vec::new();
        describe(&entry, &a, None, &mut out);
        assert!(out[0].contains("nothing painted"));
    }
}
