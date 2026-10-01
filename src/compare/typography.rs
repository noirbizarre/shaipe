//! How a render sets its lettering, against how the reference sets it. See
//! ADR 028.
//!
//! Like [`super::appearance`], nothing is measured here:
//! [`crate::analysis`] already describes each image's text lines, and this
//! lines the two descriptions up and says where they disagree. A render of
//! the right letters in the wrong place, size or colour is wrong in ways the
//! pixel error blurs together; here each is its own number and its own
//! sentence. Nothing is folded into a score.
//!
//! What the text *says* is not compared, and cannot be: Shaipe reads no text.
//! [`declared_text_findings`] covers the other half, the part of this that is
//! knowable from the SVG alone: whether the variant has text at all, and
//! whether its author said how sure they were.

use serde::Serialize;

use crate::analysis::{Orientation, TextLine, Typography};
use crate::render::DeclaredText;

use super::appearance::hex_distance;

/// Two lines are the same lettering when their boxes overlap at least this
/// much (intersection over union).
const MIN_MATCH_OVERLAP: f64 = 0.1;
/// A baseline further off than this share of the letter height, and at least
/// [`MIN_BASELINE_TOLERANCE`] pixels, is worth reporting.
const BASELINE_TOLERANCE_FRACTION: f64 = 0.15;
/// See [`BASELINE_TOLERANCE_FRACTION`].
const MIN_BASELINE_TOLERANCE: f64 = 1.0;
/// Relative difference in letter height or line length worth reporting.
const SIZE_TOLERANCE: f64 = 0.15;
/// Letter spacing further off than this share of the letter height is worth
/// reporting.
const SPACING_TOLERANCE: f64 = 0.1;
/// Mean channel difference (0-1) above which two colours are called
/// different. The same figure the appearance comparison uses.
const COLOUR_TOLERANCE: f64 = 0.1;
/// How many `<text>` elements a finding names before it says "and others".
const MAX_NAMED: usize = 5;

/// Text lines of the reference, each against the render's.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TypographyComparison {
    /// How many text lines the reference has.
    pub reference_line_count: usize,
    /// How many text lines the render has.
    pub render_line_count: usize,
    /// One entry per reference line, in the reference's order.
    pub lines: Vec<LineComparison>,
    /// Render lines that match no reference line: lettering the reference
    /// does not have.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmatched_render_lines: Vec<usize>,
    /// Only the disagreements, in words, each saying what to change. Empty
    /// when the lettering agrees.
    pub findings: Vec<String>,
}

impl TypographyComparison {
    /// Whether neither image has any lettering.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reference_line_count == 0 && self.render_line_count == 0
    }
}

/// One reference line against the render line covering most of it.
#[derive(Debug, Clone, Serialize)]
pub struct LineComparison {
    /// The reference line's id.
    pub reference_line_id: usize,
    /// The matched render line, `None` when the render sets nothing there.
    pub render_line_id: Option<usize>,
    /// Intersection over union of the two lines' boxes.
    pub overlap: f64,
    /// Whether the two run the same way.
    pub orientation_matches: Option<bool>,
    /// Render baseline minus reference baseline, in pixels, across the line:
    /// positive is lower for horizontal text. `None` unless both run the same
    /// way.
    pub baseline_offset: Option<f64>,
    /// Render letter height over reference letter height.
    pub height_ratio: Option<f64>,
    /// Render line length over reference line length, along the line.
    pub length_ratio: Option<f64>,
    /// Render letter count minus reference letter count.
    pub glyph_count_difference: Option<i64>,
    /// Render word count minus reference word count.
    pub word_count_difference: Option<i64>,
    /// Render mean letter gap minus the reference's, in pixels.
    pub letter_gap_difference: Option<f64>,
    /// Mean channel difference, 0-1, between the two lines' colours. `None`
    /// unless both are one flat colour.
    pub colour_difference: Option<f64>,
}

/// Line the two images' lettering up and report where it differs.
pub(crate) fn compare_typography(
    reference: &Typography,
    render: &Typography,
) -> TypographyComparison {
    let mut findings = Vec::new();
    let mut used = vec![false; render.lines.len()];
    let mut lines = Vec::new();

    for line in &reference.lines {
        // Ties go to the lowest id: strictly greater to displace the best.
        let mut best: Option<(usize, f64)> = None;
        for (index, candidate) in render.lines.iter().enumerate() {
            if used[index] {
                continue;
            }
            let overlap = overlap(line, candidate);
            if overlap >= MIN_MATCH_OVERLAP && best.is_none_or(|(_, top)| overlap > top) {
                best = Some((index, overlap));
            }
        }

        let entry = match best {
            Some((index, overlap)) => {
                used[index] = true;
                let entry = compare_line(line, &render.lines[index], overlap);
                describe(&entry, line, &render.lines[index], &mut findings);
                entry
            }
            None => {
                let b = line.bounding_box;
                findings.push(format!(
                    "the reference has lettering at x={}, y={} ({}x{}, {} letters) where the \
                     render sets none; add it as `<text>`",
                    b.x, b.y, b.width, b.height, line.glyph_count
                ));
                empty_entry(line.id)
            }
        };
        lines.push(entry);
    }

    let unmatched_render_lines: Vec<usize> = render
        .lines
        .iter()
        .enumerate()
        .filter(|(index, _)| !used[*index])
        .map(|(_, line)| line.id)
        .collect();
    for id in &unmatched_render_lines {
        findings.push(format!(
            "the render has lettering (line {id}) that matches none of the reference's; \
             remove it, or move it onto the reference's"
        ));
    }
    // Lines past the cap are counted but not described, so a count that
    // disagrees is still evidence.
    if reference.line_count != render.line_count
        && reference.lines.len() == reference.line_count
        && render.lines.len() == render.line_count
        && unmatched_render_lines.is_empty()
        && lines.iter().all(|l| l.render_line_id.is_some())
    {
        findings.push(format!(
            "the reference has {} lines of lettering and the render {}",
            reference.line_count, render.line_count
        ));
    }

    TypographyComparison {
        reference_line_count: reference.line_count,
        render_line_count: render.line_count,
        lines,
        unmatched_render_lines,
        findings,
    }
}

fn empty_entry(reference_line_id: usize) -> LineComparison {
    LineComparison {
        reference_line_id,
        render_line_id: None,
        overlap: 0.0,
        orientation_matches: None,
        baseline_offset: None,
        height_ratio: None,
        length_ratio: None,
        glyph_count_difference: None,
        word_count_difference: None,
        letter_gap_difference: None,
        colour_difference: None,
    }
}

/// Intersection over union of two lines' boxes.
fn overlap(a: &TextLine, b: &TextLine) -> f64 {
    let (a, b) = (a.bounding_box, b.bounding_box);
    let x =
        (i64::from(a.x + a.width).min(i64::from(b.x + b.width)) - i64::from(a.x.max(b.x))).max(0);
    let y =
        (i64::from(a.y + a.height).min(i64::from(b.y + b.height)) - i64::from(a.y.max(b.y))).max(0);
    let shared = (x * y) as f64;
    let union = f64::from(a.width) * f64::from(a.height) + f64::from(b.width) * f64::from(b.height)
        - shared;
    if union <= 0.0 { 0.0 } else { shared / union }
}

/// The line's length along its own direction.
fn length(line: &TextLine) -> f64 {
    match line.orientation {
        Orientation::Horizontal => f64::from(line.bounding_box.width),
        Orientation::Vertical => f64::from(line.bounding_box.height),
    }
}

fn compare_line(reference: &TextLine, render: &TextLine, overlap: f64) -> LineComparison {
    let same_way = reference.orientation == render.orientation;
    let mut entry = empty_entry(reference.id);
    entry.render_line_id = Some(render.id);
    entry.overlap = overlap;
    entry.orientation_matches = Some(same_way);
    if same_way && reference.baseline_edge == render.baseline_edge {
        entry.baseline_offset = Some(render.baseline - reference.baseline);
    }
    if reference.tall_height > 0.0 {
        entry.height_ratio = Some(render.tall_height / reference.tall_height);
    }
    if same_way && length(reference) > 0.0 {
        entry.length_ratio = Some(length(render) / length(reference));
    }
    entry.glyph_count_difference = Some(render.glyph_count as i64 - reference.glyph_count as i64);
    entry.word_count_difference = Some(render.words.len() as i64 - reference.words.len() as i64);
    entry.letter_gap_difference = Some(render.letter_gap.mean - reference.letter_gap.mean);
    if let (Some(a), Some(b)) = (&reference.appearance.colour, &render.appearance.colour) {
        entry.colour_difference = Some(hex_distance(a, b));
    }
    entry
}

/// Push a sentence for every way the entry is off.
fn describe(
    entry: &LineComparison,
    reference: &TextLine,
    render: &TextLine,
    findings: &mut Vec<String>,
) {
    let id = reference.id;
    if entry.orientation_matches == Some(false) {
        findings.push(format!(
            "reference line {id} runs {:?} and the render's matching line runs {:?}; rotate it",
            reference.orientation, render.orientation
        ));
        // Everything below assumes the two run the same way.
        return;
    }

    if let Some(offset) = entry.baseline_offset {
        let tolerance =
            MIN_BASELINE_TOLERANCE.max(BASELINE_TOLERANCE_FRACTION * reference.tall_height);
        if offset.abs() > tolerance {
            let direction = match (reference.orientation, offset > 0.0) {
                (Orientation::Horizontal, true) => "up",
                (Orientation::Horizontal, false) => "down",
                (Orientation::Vertical, true) => "left",
                (Orientation::Vertical, false) => "right",
            };
            findings.push(format!(
                "reference line {id}: the render's baseline is {:.1}px off; move the text {direction} by \
                 that much (`y` of the `<text>` for horizontal text)",
                offset.abs()
            ));
        }
    }

    if let Some(ratio) = entry.height_ratio
        && (ratio - 1.0).abs() > SIZE_TOLERANCE
    {
        findings.push(format!(
            "reference line {id}: the render's letters are {:.0}% {} the reference's; multiply \
             `font-size` by about {:.2}",
            ((ratio - 1.0).abs() * 100.0).round(),
            if ratio > 1.0 {
                "taller than"
            } else {
                "shorter than"
            },
            1.0 / ratio
        ));
    } else if let Some(ratio) = entry.length_ratio
        && (ratio - 1.0).abs() > SIZE_TOLERANCE
    {
        // Letters are the right height, so the line is the wrong length for
        // another reason: tracking, or a different face.
        findings.push(format!(
            "reference line {id}: the letters are the right height but the render's line is {:.0}% \
             {} the reference's; adjust `letter-spacing`, or the font is a different width",
            ((ratio - 1.0).abs() * 100.0).round(),
            if ratio > 1.0 {
                "longer than"
            } else {
                "shorter than"
            },
        ));
    }

    if let Some(difference) = entry.letter_gap_difference
        && difference.abs() > SPACING_TOLERANCE * reference.tall_height
        && entry
            .height_ratio
            .is_none_or(|r| (r - 1.0).abs() <= SIZE_TOLERANCE)
    {
        findings.push(format!(
            "reference line {id}: letter spacing is {:.1}px {} in the render; adjust `letter-spacing` by \
             about {:.1}px",
            difference.abs(),
            if difference > 0.0 { "wider" } else { "tighter" },
            -difference
        ));
    }

    if let Some(difference) = entry.glyph_count_difference
        && difference != 0
    {
        findings.push(format!(
            "reference line {id}: the render has {} {} letter shapes than the reference; the text \
             may differ, or the font joins or splits letters the reference does not",
            difference.abs(),
            if difference > 0 { "more" } else { "fewer" },
        ));
    }

    if let Some(difference) = entry.colour_difference
        && difference > COLOUR_TOLERANCE
    {
        findings.push(format!(
            "reference line {id}: the text colour is {} in the reference and {} in the render",
            reference.appearance.colour.as_deref().unwrap_or("?"),
            render.appearance.colour.as_deref().unwrap_or("?"),
        ));
    } else if reference.appearance.uniform != render.appearance.uniform {
        findings.push(format!(
            "reference line {id}: the reference's text is {} and the render's is {}",
            fill_summary(reference),
            fill_summary(render),
        ));
    }
}

fn fill_summary(line: &TextLine) -> &'static str {
    if line.appearance.uniform {
        "one colour"
    } else {
        "more than one colour or a gradient"
    }
}

/// What the variant's own `<text>` elements say about how well the lettering
/// is being reconstructed, in words.
///
/// `reference_lines` and `render_lines` are the two sides' text line counts
/// from [`TypographyComparison`]. Empty when the reference has no lettering
/// and the variant sets none.
#[must_use]
pub fn declared_text_findings(
    texts: &[DeclaredText],
    reference_lines: usize,
    render_lines: usize,
) -> Vec<String> {
    let mut findings = Vec::new();

    if reference_lines > 0 && texts.is_empty() {
        findings.push(format!(
            "the reference has {reference_lines} line(s) of lettering and the variant has no \
             `<text>`: drawn letters cannot be re-set or edited. Set the lettering you can read as \
             `<text>` in a declared font (`get_project` lists them), and keep paths only for \
             lettering no font matches, saying so"
        ));
    }
    if reference_lines == 0 && render_lines == 0 && texts.is_empty() {
        return findings;
    }

    let names = |texts: &[&DeclaredText]| {
        let mut named: Vec<String> = texts
            .iter()
            .take(MAX_NAMED)
            .map(|t| format!("\"{}\"", t.content))
            .collect();
        if texts.len() > MAX_NAMED {
            named.push("and others".to_owned());
        }
        named.join(", ")
    };

    let unmarked: Vec<&DeclaredText> = texts.iter().filter(|t| t.confidence.is_none()).collect();
    if !unmarked.is_empty() {
        findings.push(format!(
            "`<text>` without `data-shaipe-confidence`: {}. Add `high`, `medium` or `low` for how \
             sure you are of the reading and the font, and `data-shaipe-note` for what is doubtful; \
             do not leave a guess looking like a fact",
            names(&unmarked)
        ));
    }

    let doubtful: Vec<&DeclaredText> = texts
        .iter()
        .filter(|t| t.confidence.as_deref() == Some("low"))
        .collect();
    if !doubtful.is_empty() {
        findings.push(format!(
            "low-confidence text: {}. Look at the reference enlarged (`get_reference_image` with \
             the line's `bounding_box`) before keeping the reading, and tell the user it is a guess",
            names(&doubtful)
        ));
    }

    let unfonted: Vec<&DeclaredText> = texts.iter().filter(|t| t.font_family.is_none()).collect();
    if !unfonted.is_empty() {
        findings.push(format!(
            "`<text>` with no `font-family`: {}. It renders in whatever the machine has, so the \
             result is not reproducible; name a family the project declares",
            names(&unfonted)
        ));
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{
        BaselineEdge, BoundingBox, Gap, GroupAppearance, Orientation, TextLine, TextWord,
    };

    /// A horizontal line of `glyphs` letters 10px tall, starting at `x`,
    /// standing on `baseline`.
    fn line(id: usize, x: u32, baseline: f64, glyphs: usize, colour: &str) -> TextLine {
        let appearance = GroupAppearance {
            uniform: true,
            fill_kinds: vec!["flat"],
            colour: Some(colour.to_owned()),
            colours: Vec::new(),
        };
        TextLine {
            id,
            region_ids: (0..glyphs).collect(),
            bounding_box: BoundingBox {
                x,
                y: baseline as u32 - 10,
                width: glyphs as u32 * 8,
                height: 10,
            },
            orientation: Orientation::Horizontal,
            baseline,
            baseline_edge: BaselineEdge::Bottom,
            slope_degrees: 0.0,
            tall_height: 10.0,
            short_height: None,
            glyph_count: glyphs,
            mark_count: 0,
            off_baseline_count: 0,
            letter_gap: Gap {
                mean: 2.0,
                deviation: 0.0,
            },
            word_gap: None,
            words: vec![TextWord {
                region_ids: (0..glyphs).collect(),
                bounding_box: BoundingBox {
                    x,
                    y: baseline as u32 - 10,
                    width: glyphs as u32 * 8,
                    height: 10,
                },
                glyph_count: glyphs,
                appearance: appearance.clone(),
            }],
            confidence: 0.9,
            appearance,
        }
    }

    fn typography(lines: Vec<TextLine>) -> Typography {
        Typography {
            line_count: lines.len(),
            lines,
            blocks: Vec::new(),
        }
    }

    #[test]
    fn identical_lettering_has_nothing_to_report() {
        let a = typography(vec![line(0, 4, 30.0, 6, "#101010")]);
        let comparison = compare_typography(&a, &a);

        assert!(comparison.findings.is_empty(), "{:?}", comparison.findings);
        assert_eq!(comparison.lines[0].baseline_offset, Some(0.0));
    }

    #[test]
    fn a_baseline_two_pixels_low_says_which_way_to_move() {
        let reference = typography(vec![line(0, 4, 30.0, 6, "#101010")]);
        let render = typography(vec![line(0, 4, 32.0, 6, "#101010")]);
        let comparison = compare_typography(&reference, &render);

        assert_eq!(comparison.lines[0].baseline_offset, Some(2.0));
        assert_eq!(comparison.findings.len(), 1);
        assert!(
            comparison.findings[0].contains("up"),
            "{:?}",
            comparison.findings
        );
    }

    #[test]
    fn letters_too_small_say_how_much_to_scale_the_font() {
        let reference = typography(vec![line(0, 4, 30.0, 6, "#101010")]);
        let mut small = line(0, 4, 30.0, 6, "#101010");
        small.tall_height = 8.0;
        let comparison = compare_typography(&reference, &typography(vec![small]));

        assert!(
            comparison
                .findings
                .iter()
                .any(|f| f.contains("shorter") && f.contains("1.25")),
            "{:?}",
            comparison.findings
        );
    }

    #[test]
    fn a_wrong_text_colour_is_reported_with_both_colours() {
        let reference = typography(vec![line(0, 4, 30.0, 6, "#101010")]);
        let render = typography(vec![line(0, 4, 30.0, 6, "#d02020")]);
        let comparison = compare_typography(&reference, &render);

        assert!(
            comparison
                .findings
                .iter()
                .any(|f| f.contains("#101010") && f.contains("#d02020"))
        );
    }

    #[test]
    fn missing_lettering_is_reported_where_the_reference_has_it() {
        let reference = typography(vec![line(0, 4, 30.0, 6, "#101010")]);
        let comparison = compare_typography(&reference, &Typography::default());

        assert_eq!(comparison.lines[0].render_line_id, None);
        assert!(
            comparison.findings[0].contains("x=4"),
            "{:?}",
            comparison.findings
        );
    }

    #[test]
    fn lettering_the_reference_does_not_have_is_reported() {
        let render = typography(vec![line(0, 4, 30.0, 6, "#101010")]);
        let comparison = compare_typography(&Typography::default(), &render);

        assert_eq!(comparison.unmatched_render_lines, vec![0]);
        assert!(!comparison.is_empty());
    }

    #[test]
    fn artwork_without_any_lettering_compares_to_nothing() {
        let comparison = compare_typography(&Typography::default(), &Typography::default());

        assert!(comparison.is_empty());
        assert!(comparison.findings.is_empty());
    }

    #[test]
    fn each_reference_line_takes_a_different_render_line() {
        // Two stacked reference lines and one render line between them: the
        // second must not claim the render line the first already has.
        let reference = typography(vec![
            line(0, 4, 20.0, 6, "#101010"),
            line(1, 4, 34.0, 6, "#101010"),
        ]);
        let render = typography(vec![line(0, 4, 21.0, 6, "#101010")]);
        let comparison = compare_typography(&reference, &render);

        let matched = comparison
            .lines
            .iter()
            .filter(|l| l.render_line_id.is_some())
            .count();
        assert_eq!(matched, 1);
    }

    fn text(content: &str, confidence: Option<&str>, family: Option<&str>) -> DeclaredText {
        DeclaredText {
            content: content.to_owned(),
            font_family: family.map(str::to_owned),
            confidence: confidence.map(str::to_owned),
            note: None,
        }
    }

    #[test]
    fn lettering_drawn_as_paths_is_called_out_when_the_variant_has_no_text() {
        let findings = declared_text_findings(&[], 2, 0);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("no `<text>`"));
    }

    #[test]
    fn text_without_a_stated_confidence_is_asked_for_one() {
        let findings = declared_text_findings(&[text("Acme", None, Some("Fira Sans"))], 1, 1);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("\"Acme\"") && findings[0].contains("data-shaipe-confidence"));
    }

    #[test]
    fn low_confidence_text_sends_the_agent_back_to_look() {
        let findings =
            declared_text_findings(&[text("Acme", Some("low"), Some("Fira Sans"))], 1, 1);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("get_reference_image"));
    }

    #[test]
    fn text_without_a_family_is_not_reproducible() {
        let findings = declared_text_findings(&[text("Acme", Some("high"), None)], 1, 1);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("font-family"));
    }

    #[test]
    fn well_marked_text_in_a_declared_font_has_no_findings() {
        let findings =
            declared_text_findings(&[text("Acme", Some("high"), Some("Fira Sans"))], 1, 1);

        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn artwork_with_no_lettering_on_either_side_has_no_text_findings() {
        assert!(declared_text_findings(&[], 0, 0).is_empty());
    }
}
